use super::*;

impl SupervisorService {
    pub(super) async fn queue_worker_tick(&self) {
        let now = outbox::now_ms();
        let items = match self.store.queue.claim_due(now, Self::QUEUE_WORKER_CLAIM_LIMIT) {
            Ok(items) => items,
            Err(e) => {
                tracing::warn!(error = %e, "failed to claim due items from the outbox this tick");
                return;
            }
        };
        let max_attempts = self.store.queue.max_attempts();
        for item in items {
            // Shutdown abandons work in flight rather than draining it --
            // checked between every item, not only between
            // ticks, and raced into the delivery itself below, so
            // cancelling mid-tick does not wait out the rest of a claim
            // batch (up to `QUEUE_WORKER_CLAIM_LIMIT` items, each up to
            // `MANAGED_SUBSTRATE_CONNECT_TIMEOUT`) against exactly the
            // unreachable substrates this is meant never to wait on. An
            // item not yet started this tick is left claimed; its own
            // visibility timeout returns it to `Pending` on the next
            // start, same as a crashed worker.
            if self.cancellation_token.is_cancelled() {
                return;
            }
            // An item claimed `max_attempts` times without ever reaching
            // `fail`/`complete` (a worker panic or crash on every delivery)
            // would otherwise be handed
            // out forever -- `attempts` alone cannot bound it, since only
            // `fail` advances that counter. Dead-letter it through the
            // ordinary terminal path instead of attempting delivery again.
            if item.claim_count > u32::from(max_attempts) {
                self.dead_letter_poison_pill(item).await;
                continue;
            }
            tokio::select! {
                () = self.cancellation_token.cancelled() => return,
                () = self.deliver_queued_item(item) => {}
            }
        }
    }

    /// Parses a claimed item's queue key into what every path past this
    /// point needs, dead-lettering it (terminal) and returning `None` when
    /// it does not parse -- shared by `deliver_queued_item` and
    /// [`Self::dead_letter_poison_pill`].
    pub(super) fn parse_or_dead_letter(
        &self,
        item: &QueueItem,
        now: i64,
    ) -> Option<(AppInstanceId, QueueKey)> {
        let Ok(key) = item.queue_key.parse::<QueueKey>() else {
            tracing::warn!(
                queue_key = %item.queue_key,
                "outbox item carries an unparseable queue key; dead-lettering"
            );
            let _ = self.store.queue.fail(item.id, now, "unparseable queue key", true);
            return None;
        };
        let Ok(instance_id) = AppInstanceId::try_new(key.app_instance_id.clone()) else {
            let _ =
                self.store.queue.fail(item.id, now, "queue key's app instance id is invalid", true);
            return None;
        };
        Some((instance_id, key))
    }

    /// An item whose claim count alone exhausted the attempt budget,
    /// without `fail` ever being called for it -- dead-lettered through
    /// the same terminal path and alerting
    /// `fail_queued_item` already gives every other terminal reason.
    pub(super) async fn dead_letter_poison_pill(&self, item: QueueItem) {
        let now = outbox::now_ms();
        let Some((instance_id, key)) = self.parse_or_dead_letter(&item, now) else { return };
        self.fail_queued_item(
            &instance_id,
            &key,
            item.id,
            now,
            "delivery attempt budget exhausted without a recorded outcome (repeated crash or \
             panic during delivery)",
            true,
        )
        .await;
    }

    /// Replays one claimed item: reconnects to its target substrate,
    /// attempts the write again, and applies the outcome mapping:
    /// `applied`/`no-op`/`stale` complete and clear any `BindingConflict`
    /// the original transport failure raised; `conflict` completes **and**
    /// raises that same alert, exactly as the synchronous path does; a
    /// transport error retries; a callee error (the substrate reached and
    /// refused -- e.g. its target no longer exists) is terminal.
    ///
    /// Takes the same `instance_lock` the resident loop's own pass holds
    /// for the whole delivery -- without it, a queued write and a
    /// live pass write for the same instance could interleave and race
    /// this supervisor into a spurious `BindingConflict`, indistinguishable
    /// from real split-brain.
    pub(super) async fn deliver_queued_item(&self, item: QueueItem) {
        let now = outbox::now_ms();
        let Some((instance_id, key)) = self.parse_or_dead_letter(&item, now) else { return };
        let Ok(queued) = serde_json::from_slice::<outbox::QueuedBindingWrite>(&item.payload) else {
            tracing::warn!(
                queue_key = %item.queue_key,
                "outbox item carries an unparseable payload; dead-lettering"
            );
            let _ = self.store.queue.fail(item.id, now, "unparseable payload", true);
            return;
        };

        let lock = self.instance_lock(&key.app_instance_id);
        let _guard = lock.lock().await;

        let Some(client) =
            self.connect_for_queued_delivery(&item, &key, &instance_id, &queued, now).await
        else {
            return;
        };
        let outcome = client.attempt_write_bindings(queued.write.clone()).await;

        match outcome {
            Ok(outcomes) => {
                self.record_queued_write_success(&instance_id, &key, item.id, &queued, &outcomes)
                    .await;
            }
            Err(err) => {
                let failed_at = outbox::now_ms();
                // Only the narrower "the substrate answered that this
                // write's own target is gone" case is terminal here --
                // `deploy::is_callee_error` alone would also dead-letter a
                // transient, reached-and-answered error (a locked database,
                // a service still starting) that the wire protocol cannot
                // currently distinguish from "gone" by error code, only by
                // message text. Treating every callee
                // error as terminal on this path -- the only path that can
                // reach an *already-durable* item, unlike the synchronous
                // path's "decline to enqueue" -- would prematurely give up
                // on work that survived a restart specifically to be
                // retried.
                let terminal = deploy::is_target_gone_error(&err);
                self.fail_queued_item(
                    &instance_id,
                    &key,
                    item.id,
                    failed_at,
                    &err.to_string(),
                    terminal,
                )
                .await;
            }
        }
    }

    /// Resolves a claimed item to a connected actor for its target
    /// substrate, or returns `None` when there is nothing left to do this
    /// tick: the instance is gone or its inventory does not parse (both
    /// terminal), the target substrate left the inventory (terminal), the
    /// instance was retired between enqueue and delivery (completed as
    /// moot, not dead-lettered -- attempting the write would resurrect a
    /// binding the operator just released, and a `DeliveryExhausted` alert
    /// would be noise), or the connect failed (a non-terminal retry).
    async fn connect_for_queued_delivery(
        &self,
        item: &QueueItem,
        key: &QueueKey,
        instance_id: &AppInstanceId,
        queued: &outbox::QueuedBindingWrite,
        now: i64,
    ) -> Option<Arc<dyn WriteBindingsAttempt>> {
        let Ok(Some(state)) = self.store.get(&key.app_instance_id) else {
            let _ = self.store.queue.fail(item.id, now, "app instance no longer known", true);
            return None;
        };
        if state.retired {
            let _ = self.store.queue.complete(item.id);
            return None;
        }
        let Ok(inventory) = serde_json::from_str::<SupervisorInventory>(&state.inventory_json)
        else {
            let _ =
                self.store.queue.fail(item.id, now, "stored inventory-json does not parse", true);
            return None;
        };
        let Some(entry) = inventory.values().find(|e| e.did == queued.substrate_did) else {
            let _ = self.store.queue.fail(
                item.id,
                now,
                "target substrate is no longer in this instance's inventory",
                true,
            );
            return None;
        };

        match self.queue_connector.connect(entry).await {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::debug!(
                    queue_key = %item.queue_key,
                    attempt = item.attempts,
                    error = %e,
                    "queue worker delivery attempt failed to connect"
                );
                // Re-read the clock rather than reusing `now` from function
                // entry: `connect` can burn up to
                // `MANAGED_SUBSTRATE_CONNECT_TIMEOUT` (10s), and the early
                // backoff waits this feeds are sub-second -- a stale `now`
                // can put `next_attempt_at` in the past, governing the wait
                // by `queue_tick_secs` instead of the configured curve.
                let failed_at = outbox::now_ms();
                self.fail_queued_item(instance_id, key, item.id, failed_at, &e.to_string(), false)
                    .await;
                None
            }
        }
    }

    /// A queued write that reached its substrate: completes the item, then
    /// raises `BindingConflict` if the replay landed as a conflict (exactly
    /// as the synchronous path does) or clears the original transport
    /// failure's own alert now that delivery has actually converged.
    async fn record_queued_write_success(
        &self,
        instance_id: &AppInstanceId,
        key: &QueueKey,
        item_id: i64,
        queued: &outbox::QueuedBindingWrite,
        outcomes: &[BindingWriteOutcome],
    ) {
        let _ = self.store.queue.complete(item_id);
        let conflict = outcomes.iter().any(|o| matches!(o, BindingWriteOutcome::Conflict(_)));
        if conflict {
            if let Ok(true) = self.store.alerts.raise(
                instance_id,
                Some(&key.logical_ref),
                None,
                &queued.substrate_did,
                AlertKind::BindingConflict,
                &format!(
                    "a queued binding push for '{}' landed as a conflict on replay: {outcomes:?}",
                    key.logical_ref
                ),
            ) {
                self.publish_opened_alerts(
                    &key.app_instance_id,
                    &[(AlertKind::BindingConflict, key.logical_ref.clone())],
                )
                .await;
            }
        } else {
            let _ = self.store.alerts.clear(
                instance_id,
                Some(&key.logical_ref),
                &queued.substrate_did,
                AlertKind::BindingConflict,
            );
        }
    }

    /// Records a failed delivery attempt and, when the item's own budget is
    /// now exhausted, raises or refreshes the standing `DeliveryExhausted`
    /// alert with the current dead-letter count for this key -- the DLQ's
    /// whole stated purpose fails unless something surfaces it.
    pub(super) async fn fail_queued_item(
        &self,
        instance_id: &AppInstanceId,
        key: &QueueKey,
        item_id: i64,
        now: i64,
        error: &str,
        terminal: bool,
    ) {
        let Ok(outcome) = self.store.queue.fail(item_id, now, error, terminal) else { return };
        let FailOutcome::DeadLettered { pruned_keys } = outcome else { return };
        let count = self
            .store
            .queue
            .dead_letters()
            .map(|rows| rows.iter().filter(|d| d.queue_key == key.to_string()).count())
            .unwrap_or(0);
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(&key.logical_ref),
            None,
            &key.substrate_did,
            AlertKind::DeliveryExhausted,
            &format!(
                "{count} binding write(s) for '{}' exhausted their delivery attempt budget",
                key.logical_ref
            ),
        ) {
            self.publish_opened_alerts(
                &key.app_instance_id,
                &[(AlertKind::DeliveryExhausted, key.logical_ref.clone())],
            )
            .await;
        }

        // The DLQ cap just pruned these other keys' oldest dead letters --
        // if that was their *last* one, their own standing alert must
        // clear too, or a prune leaves a permanent red mark nothing can
        // ever clear. A pruned key's group is always this same instance
        // (group_key == app_instance_id, `outbox.rs`), but parsed
        // generically here rather than assumed, since this crate does not
        // enforce that pairing.
        for pruned_key in pruned_keys {
            let Ok(pruned) = pruned_key.parse::<QueueKey>() else { continue };
            let Ok(pruned_instance) = AppInstanceId::try_new(pruned.app_instance_id.clone()) else {
                continue;
            };
            self.clear_delivery_exhausted_if_empty(&pruned_instance, &pruned);
        }
    }

    /// Clears the standing `DeliveryExhausted` alert for `key` once none of
    /// its dead letters remain -- `replay`'s own clearing path.
    pub(super) fn clear_delivery_exhausted_if_empty(
        &self,
        instance_id: &AppInstanceId,
        key: &QueueKey,
    ) {
        let remaining = self
            .store
            .queue
            .dead_letters()
            .map(|rows| rows.iter().any(|d| d.queue_key == key.to_string()))
            .unwrap_or(true);
        if !remaining {
            let _ = self.store.alerts.clear(
                instance_id,
                Some(&key.logical_ref),
                &key.substrate_did,
                AlertKind::DeliveryExhausted,
            );
        }
    }
}
