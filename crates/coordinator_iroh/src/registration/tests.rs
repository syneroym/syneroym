use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::{sync::watch, task, time};

use super::*;

#[tokio::test(start_paused = true)]
async fn registration_repeats_on_its_own_timer() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = watch::channel(false);

    let register = {
        let calls = Arc::clone(&calls);
        move || {
            let calls = Arc::clone(&calls);
            async move { calls.fetch_add(1, Ordering::SeqCst) > 0 }
        }
    };

    let handle = tokio::spawn(keep_registered(register, tx));

    while calls.load(Ordering::SeqCst) < 1 {
        task::yield_now().await;
    }
    assert!(!*rx.borrow());

    time::advance(REGISTRATION_RETRY - Duration::from_secs(1)).await;
    task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    time::advance(Duration::from_secs(1)).await;
    while calls.load(Ordering::SeqCst) < 2 {
        task::yield_now().await;
    }
    assert!(*rx.borrow());

    time::advance(REGISTRATION_REFRESH - Duration::from_secs(1)).await;
    task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    time::advance(Duration::from_secs(1)).await;
    while calls.load(Ordering::SeqCst) < 3 {
        task::yield_now().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    handle.abort();
}
