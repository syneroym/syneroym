use super::*;

const NOW: u64 = 1_000_000;

#[test]
fn a_challenge_works_once() {
    let store = ChallengeStore::default();
    let c = store.issue(NOW).unwrap();
    assert_eq!(store.take(&c, NOW), Ok(()));
    assert_eq!(store.take(&c, NOW), Err(RegisterError::UnknownChallenge));
}

#[test]
fn a_challenge_expires_after_its_life() {
    let store = ChallengeStore::default();
    let old = store.issue(NOW).unwrap();
    assert_eq!(store.take(&old, NOW + CHALLENGE_LIFE_SECS), Err(RegisterError::ChallengeExpired));
    let fresh = store.issue(NOW).unwrap();
    assert_eq!(store.take(&fresh, NOW + CHALLENGE_LIFE_SECS - 1), Ok(()));
}

#[test]
fn the_oldest_challenge_is_dropped_first() {
    let store = ChallengeStore::default();
    let all: Vec<Challenge> = (0..=MAX_CHALLENGES).map(|_| store.issue(NOW).unwrap()).collect();
    assert_eq!(store.take(&all[0], NOW), Err(RegisterError::UnknownChallenge));
    assert_eq!(store.take(&all[1], NOW), Ok(()));
    assert_eq!(store.take(&all[MAX_CHALLENGES], NOW), Ok(()));
}

#[test]
fn the_store_never_holds_more_than_the_limit() {
    let store = ChallengeStore::default();
    for _ in 0..5_000 {
        store.issue(NOW).unwrap();
        assert!(store.live_len() <= MAX_CHALLENGES);
    }
}

#[test]
fn issue_and_take_does_not_grow_without_bound() {
    let store = ChallengeStore::default();
    for _ in 0..10_000 {
        let c = store.issue(NOW).unwrap();
        store.take(&c, NOW).unwrap();
        assert!(store.order_len() < 2 * MAX_CHALLENGES + 1);
    }
}

#[test]
fn ended_challenges_leave_at_the_next_issue() {
    let store = ChallengeStore::default();
    for _ in 0..10 {
        store.issue(NOW).unwrap();
    }
    store.issue(NOW + CHALLENGE_LIFE_SECS).unwrap();
    assert_eq!(store.live_len(), 1);
    assert_eq!(store.order_len(), 1);
}

#[test]
fn an_unknown_challenge_is_refused() {
    let store = ChallengeStore::default();
    let never_issued = Challenge::random().unwrap();
    assert_eq!(store.take(&never_issued, NOW), Err(RegisterError::UnknownChallenge));
}
