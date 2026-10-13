//! The HTTP half of the resolver, against small local servers. These tests
//! use the real clock: a paused clock would jump forward while a request
//! waits for the network.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    http::{StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use syneroym_core::{
    coordinator_info::{CoordinatorInfo, CoordinatorStatus, INFO_PATH, RelayInfo},
    curated_list::{CoordinatorEntry, CoordinatorTransport, CuratedList},
};
use tokio::net::TcpListener;

use super::{Http, MAX_PROBE_BODY_BYTES};

/// Serves `app` on 127.0.0.1:0 and returns its base URL.
async fn serve(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    url
}

/// A URL where nothing listens.
async fn refused_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

fn healthy() -> CoordinatorInfo {
    CoordinatorInfo {
        endpoint_addr_bytes: Vec::new(),
        substrate_id: "test".into(),
        relay_url: None,
        parent_coordinator_url: None,
        status: CoordinatorStatus::Healthy,
        identity: None,
        connections: None,
        tls: None,
        relay: Some(RelayInfo { online: true, clients: None, max_clients: None }),
        registry: None,
    }
}

fn counted(hits: &Arc<AtomicUsize>) -> Arc<AtomicUsize> {
    hits.clone()
}

#[tokio::test]
async fn probe_reads_the_info_path() {
    let app = Router::new().route(INFO_PATH, get(|| async { Json(healthy()) }));
    let url = serve(app).await;

    let info = Http::new().unwrap().probe(&url).await.unwrap();

    assert_eq!(info.status, CoordinatorStatus::Healthy);
    assert!(info.relay.is_some_and(|r| r.online));
}

#[tokio::test]
async fn probe_does_not_follow_a_redirect() {
    let hits = Arc::new(AtomicUsize::new(0));
    let target_hits = counted(&hits);
    let app = Router::new()
        .route(INFO_PATH, get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/elsewhere")]) }))
        .route(
            "/elsewhere",
            get(move || async move {
                target_hits.fetch_add(1, Ordering::SeqCst);
                Json(healthy())
            }),
        );
    let url = serve(app).await;

    let answer = Http::new().unwrap().probe(&url).await;

    assert!(answer.is_err(), "a redirect is an error, not an answer");
    assert_eq!(hits.load(Ordering::SeqCst), 0, "the redirect target was not requested");
}

#[tokio::test]
async fn probe_refuses_a_very_long_answer() {
    let big = "x".repeat(MAX_PROBE_BODY_BYTES + 1);
    let app =
        Router::new().route(INFO_PATH, get(move || async move { big.clone().into_response() }));
    let url = serve(app).await;

    let err = Http::new().unwrap().probe(&url).await.unwrap_err();

    assert!(format!("{err:#}").contains("longer than"), "{err:#}");
}

fn one_entry_list() -> CuratedList<CoordinatorEntry> {
    CuratedList {
        max_age_secs: 60,
        entries: vec![CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "http://relay.test".into(),
            info_url: "http://info.test".into(),
            operator: "test".into(),
        }],
    }
}

#[tokio::test]
async fn fetch_tries_a_refused_connection_three_times_until_the_registry_answers() {
    let http = Http::new().unwrap();
    let url = refused_url().await;

    let start = Instant::now();
    assert!(http.fetch_list(&url).await.is_err());

    // Waits of 1 s and 2 s between three tries.
    assert!(start.elapsed() >= Duration::from_secs(3), "{:?}", start.elapsed());
}

#[tokio::test]
async fn fetch_does_not_try_again_after_an_error_status() {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = counted(&hits);
    let app = Router::new().route(
        "/coordinators",
        get(move || async move {
            counter.fetch_add(1, Ordering::SeqCst);
            StatusCode::SERVICE_UNAVAILABLE
        }),
    );
    let url = serve(app).await;

    assert!(Http::new().unwrap().fetch_list(&url).await.is_err());

    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fetch_does_not_retry_a_refused_connection_once_the_registry_has_answered() {
    let app = Router::new().route("/coordinators", get(|| async { Json(one_entry_list()) }));
    let up = serve(app).await;
    let down = refused_url().await;
    let http = Http::new().unwrap();
    assert!(http.fetch_list(&up).await.is_ok());

    let start = Instant::now();
    assert!(http.fetch_list(&down).await.is_err());

    assert!(start.elapsed() < Duration::from_secs(1), "{:?}", start.elapsed());
}
