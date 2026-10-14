//! The challenges of the challenge form: random, short-lived, single use,
//! and bounded in number.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Mutex, PoisonError},
};

use anyhow::Result;

use crate::{error::RegisterError, proof::Challenge};

/// The most challenges kept at one time. When full, the oldest goes first.
pub const MAX_CHALLENGES: usize = 1_000;
/// How long a challenge works after it is issued.
pub const CHALLENGE_LIFE_SECS: u64 = 60;

/// The challenges issued and not yet used, in memory.
#[derive(Debug, Default)]
pub struct ChallengeStore {
    inner: Mutex<Queue>,
}

#[derive(Debug, Default)]
struct Queue {
    /// Issue order. May still hold challenges that were taken; they leave
    /// when their life ends or at the next compaction.
    order: VecDeque<(Challenge, u64)>,
    /// Challenge -> issue time.
    live: HashMap<Challenge, u64>,
}

impl Queue {
    fn drop_ended(&mut self, now_secs: u64) {
        while let Some(&(old, at)) = self.order.front() {
            if now_secs < at.saturating_add(CHALLENGE_LIFE_SECS) {
                break;
            }
            self.order.pop_front();
            if self.live.get(&old) == Some(&at) {
                self.live.remove(&old);
            }
        }
    }

    fn drop_oldest_live(&mut self) {
        while let Some((old, _)) = self.order.pop_front() {
            if self.live.remove(&old).is_some() {
                break;
            }
        }
    }

    /// Taken challenges stay in `order` until their life ends. Remove them
    /// here, so `order` cannot grow without bound under fast issue and take.
    fn compact(&mut self) {
        let live = &self.live;
        self.order.retain(|(c, _)| live.contains_key(c));
    }
}

impl ChallengeStore {
    /// Issue a new challenge at `now_secs`.
    pub fn issue(&self, now_secs: u64) -> Result<Challenge> {
        let challenge = Challenge::random()?;
        let mut q = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        q.drop_ended(now_secs);
        if q.live.len() >= MAX_CHALLENGES {
            q.drop_oldest_live();
        }
        if q.order.len() >= 2 * MAX_CHALLENGES {
            q.compact();
        }
        q.order.push_back((challenge, now_secs));
        q.live.insert(challenge, now_secs);
        Ok(challenge)
    }

    /// Use up `challenge`. It works once, and only within its life.
    pub(crate) fn take(&self, challenge: &Challenge, now_secs: u64) -> Result<(), RegisterError> {
        let mut q = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match q.live.remove(challenge) {
            None => Err(RegisterError::UnknownChallenge),
            Some(at) if now_secs >= at.saturating_add(CHALLENGE_LIFE_SECS) => {
                Err(RegisterError::ChallengeExpired)
            }
            Some(_) => Ok(()),
        }
    }

    #[cfg(test)]
    fn live_len(&self) -> usize {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner).live.len()
    }

    #[cfg(test)]
    fn order_len(&self) -> usize {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner).order.len()
    }
}

#[cfg(test)]
mod tests;
