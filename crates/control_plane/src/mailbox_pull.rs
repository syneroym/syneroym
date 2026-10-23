//! The node owner's hook into the mailbox receiver of this substrate.

use std::fmt;

use syneroym_core::mailbox::PullSummary;

/// Implemented by the router's mailbox puller. Held as a `Weak`: the puller
/// holds the route handler, which holds this crate's service.
#[async_trait::async_trait]
pub trait MailboxPullControl: Send + Sync + fmt::Debug {
    /// Runs one pass now, after any pass that is running, and answers what
    /// it did.
    async fn pull_now(&self) -> PullSummary;
    /// Asks for a pass soon. Does not wait.
    fn wake(&self);
    /// Removes the service's queue at `mailbox` (one `unregister`, made as
    /// the service's current instance). Best effort; never fails.
    async fn release(&self, service_id: &str, mailbox: &str);
}
