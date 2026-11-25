//! The HTTP side: `GET|PUT /v1/head`, `GET /v1/info`, all behind `Authorization: Bearer <secret>`.
//!
//! Nothing here logs a request body, a header or the secret.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::store::{Expected, HubStore, MAX_ENVELOPE_LEN, PutError, peek_header};

const PROTOCOL: u32 = 1;
/// A little room for framing, so a body of exactly the limit is never cut off by accident.
const BODY_SLACK: usize = 4096;

struct Shared {
    store: Arc<HubStore>,
    secret_sha256: [u8; 32],
}

pub fn router(store: Arc<HubStore>, secret_sha256: [u8; 32]) -> Router {
    Router::new()
        .route("/v1/head", get(get_head).put(put_head))
        .route("/v1/info", get(info))
        .with_state(Arc::new(Shared {
            store,
            secret_sha256,
        }))
}

/// Binds `addr`, serves `router` on it in a background task and returns the bound address
/// (use port 0 in tests).
pub async fn serve(addr: SocketAddr, router: Router) -> std::io::Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok(bound)
}

/// Resolves when the process is asked to stop: Ctrl+C (SIGINT) everywhere, and SIGTERM on Unix
/// (what `docker stop`, `podman stop` and systemd send; the hub runs as PID 1 in its container,
/// where a signal without a handler is ignored). A handler that cannot be installed never
/// resolves, so a failure to register does not stop the hub by itself.
pub async fn shutdown_signal() {
    async fn interrupt() {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    () = interrupt() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => interrupt().await,
        }
    }
    #[cfg(not(unix))]
    interrupt().await;
}

fn status(code: StatusCode) -> Response {
    code.into_response()
}

fn authorised(shared: &Shared, headers: &HeaderMap) -> bool {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    bool::from(digest.ct_eq(&shared.secret_sha256))
}

fn unauthorised() -> Response {
    let mut r = status(StatusCode::UNAUTHORIZED);
    r.headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    r
}

async fn info(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorised(&shared, &headers) {
        return unauthorised();
    }
    let body = format!("{{\"protocol\":{PROTOCOL},\"maxBytes\":{MAX_ENVELOPE_LEN}}}");
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

async fn get_head(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorised(&shared, &headers) {
        return unauthorised();
    }
    let store = Arc::clone(&shared.store);
    match tokio::task::spawn_blocking(move || store.head()).await {
        Ok(Ok(Some((seq, bytes)))) => (
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
                (header::ETAG, format!("\"{seq}\"")),
            ],
            bytes,
        )
            .into_response(),
        Ok(Ok(None)) => status(StatusCode::NOT_FOUND),
        _ => status(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// `If-Match: "<seq>"` or `If-None-Match: *`; exactly one of them.
fn precondition(headers: &HeaderMap) -> Result<Expected, StatusCode> {
    let if_match = headers.get(header::IF_MATCH);
    let none_match = headers.get(header::IF_NONE_MATCH);
    match (if_match, none_match) {
        (None, None) => Err(StatusCode::PRECONDITION_REQUIRED),
        (Some(_), Some(_)) => Err(StatusCode::BAD_REQUEST),
        (None, Some(v)) => match v.to_str() {
            Ok("*") => Ok(Expected::Empty),
            _ => Err(StatusCode::BAD_REQUEST),
        },
        (Some(v), None) => {
            let s = v.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
            let n = s
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or(StatusCode::BAD_REQUEST)?;
            Ok(Expected::Seq(n))
        }
    }
}

/// Reads at most `limit + BODY_SLACK` bytes; `None` when the body is larger (the read is aborted,
/// the rest is never pulled from the connection).
async fn read_capped(mut body: Body, limit: usize) -> Result<Option<Vec<u8>>, ()> {
    let cap = limit + BODY_SLACK;
    let mut out: Vec<u8> = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| ())?;
        if let Ok(data) = frame.into_data() {
            if out.len() + data.len() > cap {
                return Ok(None);
            }
            out.extend_from_slice(&data);
        }
    }
    Ok(Some(out))
}

async fn put_head(State(shared): State<Arc<Shared>>, headers: HeaderMap, body: Body) -> Response {
    if !authorised(&shared, &headers) {
        return unauthorised();
    }
    let expected = match precondition(&headers) {
        Ok(e) => e,
        Err(code) => return status(code),
    };
    if let Some(len) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        && len > MAX_ENVELOPE_LEN as u64
    {
        return status(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let bytes = match read_capped(body, MAX_ENVELOPE_LEN).await {
        Ok(Some(b)) => b,
        Ok(None) => return status(StatusCode::PAYLOAD_TOO_LARGE),
        Err(()) => return status(StatusCode::BAD_REQUEST),
    };
    let Some((group, seq)) = peek_header(&bytes) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let store = Arc::clone(&shared.store);
    let created = expected == Expected::Empty;
    let result = tokio::task::spawn_blocking(move || store.put(expected, seq, group, &bytes)).await;
    match result {
        Ok(Ok(())) if created => status(StatusCode::CREATED),
        Ok(Ok(())) => status(StatusCode::NO_CONTENT),
        Ok(Err(PutError::Stale { current })) => {
            let mut r = status(StatusCode::PRECONDITION_FAILED);
            if let Some(c) = current
                && let Ok(v) = HeaderValue::from_str(&format!("\"{c}\""))
            {
                r.headers_mut().insert(header::ETAG, v);
            }
            r
        }
        Ok(Err(PutError::SeqNotHigher | PutError::WrongGroup)) => status(StatusCode::CONFLICT),
        Ok(Err(PutError::TooLarge)) => status(StatusCode::PAYLOAD_TOO_LARGE),
        _ => status(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
