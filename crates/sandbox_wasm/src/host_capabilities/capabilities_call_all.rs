use std::time::Duration;

/// Proof token certifying that wait credit was measured and granted exclusively
/// by the batched call execution path.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct WaitCredit(pub(super) Duration);

impl WaitCredit {
    /// Returns the credited duration.
    #[allow(dead_code)]
    #[must_use]
    pub(crate) fn duration(&self) -> Duration {
        self.0
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn for_test(d: Duration) -> Self {
        Self(d)
    }
}
