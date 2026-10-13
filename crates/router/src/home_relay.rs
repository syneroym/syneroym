//! The home relay of this substrate's Iroh endpoint, set at run time.
//!
//! The endpoint starts with an empty relay map. `HomeRelay::set` puts one
//! relay in the map, waits until iroh confirms it as the home relay, and only
//! then shows the URL to subscribers. The URL is never read back from iroh:
//! after the last relay is removed, iroh 0.97 still lists it in
//! `Endpoint::addr`.
//!
//! "Confirmed" means iroh chose the relay after a net-report probe reached
//! it. iroh does this before the relay connection opens and before the relay
//! checks access, so a confirmed relay may still refuse this node. Do not use
//! confirmation as proof of access.

use std::{sync::Arc, time::Duration};

use iroh::{Endpoint, RelayConfig, RelayUrl, Watcher};
use tokio::{
    sync::{Mutex, watch},
    time,
};
use tracing::warn;

/// How long `HomeRelay::set` waits for iroh to confirm a relay. A switch
/// needs two net reports, and each can take up to 5 s when the relay does not
/// serve QUIC address discovery on the port iroh probes.
pub const HOME_RELAY_CONFIRM_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum HomeRelayError {
    #[error("relay {url} was not confirmed as the home relay within {timeout:?}")]
    NotConfirmed { url: RelayUrl, timeout: Duration },
    #[error("the Iroh endpoint is closed")]
    EndpointClosed,
}

/// A cheap, cloneable handle to the home relay of one endpoint.
#[derive(Debug, Clone)]
pub struct HomeRelay {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    endpoint: Endpoint,
    /// The confirmed home relay. `None` until the first `set` succeeds.
    confirmed: watch::Sender<Option<RelayUrl>>,
    /// The relay that is in the relay map now. The lock also makes switches
    /// run one at a time, so two callers never interleave insert and remove.
    /// It differs from `confirmed` only after a first `set` that was not
    /// confirmed: then the relay stays in the map so iroh keeps trying it.
    in_map: Mutex<Option<RelayUrl>>,
    confirm_timeout: Duration,
}

impl HomeRelay {
    /// `endpoint` must have been built with an empty relay map.
    #[must_use]
    pub fn new(endpoint: Endpoint, confirm_timeout: Duration) -> Self {
        let (confirmed, _) = watch::channel(None);
        Self {
            inner: Arc::new(Inner {
                endpoint,
                confirmed,
                in_map: Mutex::new(None),
                confirm_timeout,
            }),
        }
    }

    /// The confirmed home relay URL, and every later change of it.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<Option<RelayUrl>> {
        self.inner.confirmed.subscribe()
    }

    /// The confirmed home relay URL, or `None` if no relay is confirmed yet.
    #[must_use]
    pub fn current_url(&self) -> Option<RelayUrl> {
        self.inner.confirmed.borrow().clone()
    }

    /// Makes `url` the home relay.
    ///
    /// # Cancellation
    /// If the returned future is dropped while waiting for confirmation (such
    /// as during a timeout or `select!`), no rollback runs. The new relay stays
    /// in the map while the watch channel still names the old relay. Callers
    /// should run `set` to completion without dropping the future.
    ///
    /// # Errors
    /// `NotConfirmed` when iroh does not confirm the relay in time. A switch
    /// from a confirmed relay is then rolled back; a first relay stays in the
    /// map and the caller may call `set` again. `EndpointClosed` when the
    /// endpoint is closed.
    pub async fn set(&self, url: RelayUrl) -> Result<(), HomeRelayError> {
        let mut in_map = self.inner.in_map.lock().await;
        let endpoint = &self.inner.endpoint;
        if endpoint.is_closed() {
            return Err(HomeRelayError::EndpointClosed);
        }
        let confirmed = self.inner.confirmed.borrow().clone();
        if confirmed.as_ref() == Some(&url) {
            return Ok(());
        }
        // A retry of a first relay that is already in the map only waits again.
        if in_map.as_ref() != Some(&url) {
            replace_relay(endpoint, in_map.as_ref(), &url).await;
            *in_map = Some(url.clone());
        }
        match wait_until_home(endpoint, &url, self.inner.confirm_timeout).await {
            Confirm::Home => {
                self.inner.confirmed.send_replace(Some(url));
                Ok(())
            }
            Confirm::Closed => Err(HomeRelayError::EndpointClosed),
            Confirm::TimedOut => self.handle_timeout(endpoint, &mut in_map, confirmed, url).await,
        }
    }

    async fn handle_timeout(
        &self,
        endpoint: &Endpoint,
        in_map: &mut Option<RelayUrl>,
        confirmed: Option<RelayUrl>,
        url: RelayUrl,
    ) -> Result<(), HomeRelayError> {
        // iroh keeps its old home relay when the new one does not answer,
        // so the old relay still works: put it back.
        if let Some(old) = confirmed {
            replace_relay(endpoint, Some(&url), &old).await;
            if !endpoint.addr().relay_urls().any(|u| *u == old) {
                // Not confirmed: very unlikely, logged so it can be seen.
                warn!(
                    relay_url = %old,
                    "rolled back to the old relay, but iroh does not list it as the home relay"
                );
            }
            *in_map = Some(old);
        }
        Err(HomeRelayError::NotConfirmed { url, timeout: self.inner.confirm_timeout })
    }

    /// Waits until the endpoint is closed. A background task uses it to stop.
    pub async fn closed(&self) {
        self.inner.endpoint.closed().await;
    }

    /// Removes a relay that a `set` put in the map but iroh never confirmed,
    /// so the relay map is empty again. Does nothing when a relay is
    /// confirmed.
    pub async fn clear_unconfirmed(&self) {
        // Take the lock first, as `set` does, so this never runs in the
        // middle of a `set`.
        let mut in_map = self.inner.in_map.lock().await;
        if self.inner.confirmed.borrow().is_some() {
            return;
        }
        if let Some(url) = in_map.take() {
            self.inner.endpoint.remove_relay(&url).await;
        }
    }

    #[cfg(test)]
    pub(crate) async fn in_map(&self) -> Option<RelayUrl> {
        self.inner.in_map.lock().await.clone()
    }
}

/// Inserts `new`, then removes `old`, so the map is never empty during a
/// switch. With both in the map, iroh keeps the old home relay; removing the
/// old relay starts the net report that picks the new one.
async fn replace_relay(endpoint: &Endpoint, old: Option<&RelayUrl>, new: &RelayUrl) {
    endpoint.insert_relay(new.clone(), Arc::new(RelayConfig::from(new.clone()))).await;
    if let Some(old) = old.filter(|old| *old != new) {
        endpoint.remove_relay(old).await;
    }
}

enum Confirm {
    Home,
    Closed,
    TimedOut,
}

async fn wait_until_home(endpoint: &Endpoint, url: &RelayUrl, timeout: Duration) -> Confirm {
    let mut addr = endpoint.watch_addr();
    let home = async {
        loop {
            if addr.get().relay_urls().any(|u| u == url) {
                return Confirm::Home;
            }
            if addr.updated().await.is_err() {
                return Confirm::Closed;
            }
        }
    };
    // The watcher does not end when the endpoint closes, only when the last
    // clone is dropped, and this handle holds one.
    let wait = async {
        tokio::select! {
            confirm = home => confirm,
            () = endpoint.closed() => Confirm::Closed,
        }
    };
    time::timeout(timeout, wait).await.unwrap_or(Confirm::TimedOut)
}

#[cfg(test)]
mod tests;
