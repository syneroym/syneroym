#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Mini-app demo guest service library
//!
//! Runs a mock sandboxed client library exposing basic network interfaces.

use std::{
    fs,
    future::Future,
    io::Cursor,
    net::SocketAddr,
    path::Path as StdPath,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    extract::{
        Json, Multipart, Path, Request, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, Uri, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use broadcast::Sender;
use chrono::Utc;
use clap::Parser;
use header::CONTENT_TYPE;
use hyper::{Request as HyperRequest, body::Incoming, service};
use hyper_util::server::conn::auto::Builder;
use rusqlite::{Connection, params};
use rust_embed::Embed;
use rustls::ServerConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};
use tokio::{
    fs as tokio_fs,
    net::TcpListener,
    sync::{broadcast, mpsc},
    task,
};
use tokio_rustls::TlsAcceptor;
use tracing_subscriber::fmt;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Service name to display
    #[arg(long, default_value = "demo1-instance0")]
    pub service_name: String,

    /// Port to listen on
    #[arg(long, default_value_t = 3000)]
    pub port: u16,

    /// HTTPS port to listen on
    #[arg(long, default_value_t = 3001)]
    pub https_port: u16,

    /// Data directory
    #[arg(long, default_value = "data")]
    pub data_dir: String,
}

#[derive(Clone)]
struct AppState {
    service_name: String,
    data_dir: String,
    // Connection is not Sync, so we need Mutex.
    // We use std::sync::Mutex because we are inside spawn_blocking mostly,
    // and rusqlite is blocking.
    conn: Arc<Mutex<Connection>>,
    tx: Sender<String>,
}

#[derive(Embed)]
#[folder = "static/"]
struct Assets;

async fn index_handler(State(state): State<Arc<AppState>>) -> Html<String> {
    Html(format!(
        "<h1>Hello world from {}</h1><p><a href='/comments'>Comments etc.</a></p><p><a \
         href='/page1.html'>Static Page 1</a></p>",
        state.service_name
    ))
}

async fn comments_page_handler() -> impl IntoResponse {
    match Assets::get("dist/index.html") {
        Some(content) => Html(String::from_utf8_lossy(&content.data).to_string()).into_response(),
        None => (StatusCode::NOT_FOUND, "Comments page not found. Did you build the client?")
            .into_response(),
    }
}

#[derive(Deserialize)]
struct CreateComment {
    text: String,
}

#[derive(Serialize)]
struct Comment {
    id: i64,
    text: String,
}

async fn get_recent_comments(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let state = state.clone();
    let result = task::spawn_blocking(move || {
        let conn = state.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, text FROM comments ORDER BY id DESC LIMIT 5")
            .map_err(|e| e.to_string())?;

        let comments_iter = stmt
            .query_map([], |row| Ok(Comment { id: row.get(0)?, text: row.get(1)? }))
            .map_err(|e| e.to_string())?;

        let mut comments = Vec::new();
        for comment in comments_iter {
            comments.push(comment.map_err(|e| e.to_string())?);
        }
        Ok::<_, String>(comments)
    })
    .await;

    match result {
        Ok(Ok(comments)) => Json(comments).into_response(),
        Ok(Err(e)) => {
            eprintln!("Database query error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
        Err(e) => {
            eprintln!("Join error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

async fn save_comment(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateComment>,
) -> impl IntoResponse {
    let state_for_db = state.clone();
    let text = payload.text.clone();

    // Offload blocking DB operation to a thread pool
    let result = task::spawn_blocking(move || {
        let conn = state_for_db.conn.lock().unwrap();
        conn.execute("INSERT INTO comments (text) VALUES (?)", params![text])
    })
    .await;

    match result {
        Ok(Ok(_)) => {
            let _ = state.tx.send(Utc::now().to_rfc3339());
            StatusCode::CREATED
        }
        Ok(Err(e)) => {
            eprintln!("Database error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
        Err(e) => {
            eprintln!("Join error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[derive(Serialize)]
struct FileInfo {
    name: String,
    size: u64,
}

async fn list_files(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut files = Vec::new();
    // Ensure directory exists
    if let Ok(mut entries) = tokio_fs::read_dir(&state.data_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(metadata) = entry.metadata().await
                && metadata.is_file()
            {
                // Filter out the database file
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.ends_with(".db") {
                    files.push(FileInfo { name, size: metadata.len() });
                }
            }
        }
    }
    Json(files)
}

async fn upload_file(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    while let Ok(Some(field)) = multipart.next_field().await {
        if let Some(file_name) = field.file_name() {
            let file_name = file_name.to_string();
            // Simple sanitization
            if file_name.contains("..") || file_name.contains('/') || file_name.contains('\\') {
                continue;
            }

            if let Ok(data) = field.bytes().await {
                let file_path = StdPath::new(&state.data_dir).join(&file_name);
                if let Err(e) = tokio_fs::write(&file_path, data).await {
                    eprintln!("Failed to write file: {e}");
                    return StatusCode::INTERNAL_SERVER_ERROR;
                }
            }
        }
    }
    StatusCode::CREATED
}

async fn download_file(
    State(state): State<Arc<AppState>>,
    Path(filename): Path<String>,
) -> impl IntoResponse {
    // Security check: ensure path is inside data_dir
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return (StatusCode::BAD_REQUEST, "Invalid filename").into_response();
    }

    let file_path = StdPath::new(&state.data_dir).join(&filename);

    match tokio_fs::read(&file_path).await {
        Ok(file) => {
            let mime = mime_guess::from_path(&filename).first_or_octet_stream();
            ([(CONTENT_TYPE, mime.as_ref())], file).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "File not found").into_response(),
    }
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WsMessage {
    comment_update_timestamp: Option<String>,
    recd_msg: Option<String>,
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.tx.subscribe();
    loop {
        tokio::select! {
            Ok(msg) = rx.recv() => {
                let ws_msg = WsMessage {
                    comment_update_timestamp: Some(msg),
                    recd_msg: None,
                };
                if let Ok(json) = serde_json::to_string(&ws_msg)
                    && socket.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
            }
            Some(msg) = socket.recv() => {
                match msg {
                    Ok(Message::Text(text)) => {
                        println!("[{}] Received: {}", chrono::Utc::now(), text);
                        let ws_msg = WsMessage {
                            comment_update_timestamp: None,
                            recd_msg: Some(format!("[{}] Received: {}", chrono::Utc::now(), text).to_string()),
                        };
                         if let Ok(json) = serde_json::to_string(&ws_msg)
                            && socket.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                    }
                    Ok(Message::Close(_)) | Err(_) => {
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(CONTENT_TYPE, mime.as_ref())], content.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    }
}

async fn print_request_log(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    debug!("-> {} {}", method, uri);
    let res = next.run(req).await;
    debug!("<- {} {} status={}", method, uri, res.status());
    res
}

use tracing::{debug, info};

pub async fn real_main() {
    fmt::init();
    let args = Args::parse();
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));

    let (_tx, mut rx) = mpsc::channel::<()>(1);
    run_server(args, addr, async move {
        let _ = rx.recv().await;
    })
    .await
    .unwrap();
}

pub async fn run_server(
    args: Args,
    addr: SocketAddr,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(&args.data_dir)?;
    let db_path = std::path::Path::new(&args.data_dir).join("comments.db");
    let conn = Connection::open(&db_path)?;

    // Run migration
    conn.execute("CREATE TABLE IF NOT EXISTS comments (id INTEGER PRIMARY KEY, text TEXT)", [])?;

    let (tx, _rx) = broadcast::channel(100);

    let state = Arc::new(AppState {
        service_name: args.service_name,
        data_dir: args.data_dir.clone(),
        conn: Arc::new(Mutex::new(conn)),
        tx,
    });

    // Build our application
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/comments", get(comments_page_handler))
        .route("/api/comments", post(save_comment).get(get_recent_comments))
        .route("/api/files", post(upload_file).get(list_files))
        .route("/api/files/{filename}", get(download_file))
        .route("/ws", get(websocket_handler))
        .fallback(static_handler)
        .layer(middleware::from_fn(print_request_log))
        .with_state(state);

    info!("listening on http://{}", addr);
    let http_listener = TcpListener::bind(addr).await?;

    // HTTPS Setup
    let https_addr = SocketAddr::from(([0, 0, 0, 0], args.https_port));
    info!("listening on https://{}", https_addr);
    let https_listener = TcpListener::bind(https_addr).await?;

    let cert_pem = include_str!("test_cert.pem");
    let key_pem = include_str!("test_key.pem");

    let mut cert_reader = Cursor::new(cert_pem);
    let mut key_reader = Cursor::new(key_pem);

    use rustls_pki_types::pem::PemObject;

    let certs: Vec<CertificateDer> =
        CertificateDer::pem_reader_iter(&mut cert_reader).collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_reader(&mut key_reader)
        .map_err(|e| format!("no private key found in test_key.pem: {e}"))?;

    let tls_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("invalid certificate: {e}"))?;

    let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    let mut http_shutdown_rx = shutdown_tx.subscribe();
    let mut https_shutdown_rx = shutdown_tx.subscribe();

    let http_app = app.clone();
    let http_server = tokio::spawn(async move {
        axum::serve(http_listener, http_app)
            .with_graceful_shutdown(async move {
                let _ = http_shutdown_rx.recv().await;
            })
            .await
            .unwrap();
    });

    let https_app = app;
    let https_server = tokio::spawn(async move {
        use hyper_util::rt::{TokioExecutor, TokioIo};
        use tower_service::Service;

        loop {
            let (tcp_stream, _remote_addr) = tokio::select! {
                res = https_listener.accept() => res.unwrap(),
                _ = https_shutdown_rx.recv() => break,
            };

            let tls_acceptor = tls_acceptor.clone();
            let app = https_app.clone();

            tokio::spawn(async move {
                let tls_stream = match tls_acceptor.accept(tcp_stream).await {
                    Ok(tls_stream) => tls_stream,
                    Err(err) => {
                        eprintln!("failed to perform tls handshake: {err:#}");
                        return;
                    }
                };

                let io = TokioIo::new(tls_stream);
                let hyper_service = service::service_fn(move |request: HyperRequest<Incoming>| {
                    app.clone().call(request)
                });

                if let Err(err) = Builder::new(TokioExecutor::new())
                    .serve_connection_with_upgrades(io, hyper_service)
                    .await
                {
                    eprintln!("failed to serve connection: {err:#}");
                }
            });
        }
    });

    // Wait for shutdown signal
    shutdown.await;
    let _ = shutdown_tx.send(());

    let _ = tokio::join!(http_server, https_server);

    Ok(())
}
