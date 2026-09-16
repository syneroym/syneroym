use super::*;

static SW_JS_GZ: OnceLock<Option<Vec<u8>>> = OnceLock::new();
static PEER_PROXY_JS_GZ: OnceLock<Option<Vec<u8>>> = OnceLock::new();

fn compress_gzip(data: &str) -> Result<Vec<u8>, IoError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data.as_bytes())?;
    encoder.finish()
}
fn serve_cached_js(
    js_content: &'static str,
    name: &str,
    cache: &OnceLock<Option<Vec<u8>>>,
) -> Response {
    let gzipped_opt = cache.get_or_init(|| {
        compress_gzip(js_content).inspect_err(|e| error!("Failed to compress {}: {}", name, e)).ok()
    });

    let mut response = match gzipped_opt {
        Some(gzipped) => {
            let mut res = gzipped.clone().into_response();
            res.headers_mut()
                .insert(header::CONTENT_ENCODING, header::HeaderValue::from_static("gzip"));
            res
        }
        None => {
            let mut res = js_content.as_bytes().to_vec().into_response();
            res.headers_mut().insert(
                header::HeaderName::from_static("x-compression-failed"),
                header::HeaderValue::from_static("true"),
            );
            res
        }
    };

    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, header::HeaderValue::from_static("application/javascript"));
    response
}

pub(super) async fn handle_sw() -> impl IntoResponse {
    info!("Serving sw.js to client");
    let sw_js = include_str!(concat!(env!("OUT_DIR"), "/sw.js"));
    let mut response = serve_cached_js(sw_js, "sw.js", &SW_JS_GZ);
    response.headers_mut().insert(
        header::HeaderName::from_static("service-worker-allowed"),
        header::HeaderValue::from_static("/"),
    );
    response
}

pub(super) async fn handle_peer_proxy_js() -> impl IntoResponse {
    let js = include_str!(concat!(env!("OUT_DIR"), "/peer-proxy.js"));
    serve_cached_js(js, "peer-proxy.js", &PEER_PROXY_JS_GZ)
}
