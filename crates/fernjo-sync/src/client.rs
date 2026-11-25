//! The blocking HTTP client for the sync hub.
//!
//! HTTPS only (plain `http://` only to the literal loopback addresses, and only in debug builds),
//! trust is the system roots or exactly one pinned CA from the pairing code, redirects are never
//! followed, every call has a connect and a total timeout, and a response body is read in bounded
//! steps: more than [`MAX_ENVELOPE_LEN`] bytes is `hub_too_large` and the rest is never pulled from
//! the socket. The hub secret is held as a `SecretString` and only ever sent as the bearer header.

use std::fmt;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use ureq::tls::{Certificate, PemItem, RootCerts, TlsConfig, TlsProvider, parse_pem};

use crate::envelope::{MAX_ENVELOPE_LEN, peek_header};
use crate::error::SyncError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);
const INFO_MAX_LEN: usize = 4096;

/// Where the hub is and how to talk to it.
pub struct HubConfig {
    pub url: String,
    pub secret: SecretString,
    /// A pinned CA certificate (PEM). With it only that CA is trusted; without it the system roots.
    pub ca_pem: Option<String>,
}

impl fmt::Debug for HubConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HubConfig")
            .field("url", &self.url)
            .field("secret", &"<redacted>")
            .field("ca_pem", &self.ca_pem.as_ref().map(|_| "<certificate>"))
            .finish()
    }
}

/// What the writer believes the hub holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expected {
    Empty,
    Seq(u64),
}

/// The answer of `GET /v1/info`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubInfo {
    pub protocol: u32,
    pub max_bytes: u64,
}

/// Whether plain `http://` to a literal loopback address is accepted: in debug builds only, which
/// is what the tests run (the in-process hub speaks plain HTTP on loopback). A release build
/// talks HTTPS only, so another local user who binds the port while a loopback hub is down never
/// receives the hub secret.
const ALLOW_PLAIN_LOOPBACK: bool = cfg!(debug_assertions);

/// Accepts `https://...`, and, in debug builds, `http://` for the literal addresses `127.0.0.1`
/// and `[::1]`. `localhost` is not accepted, and neither is a user name, a query or a fragment in
/// the address.
pub(crate) fn check_hub_url(url: &str) -> Result<(), SyncError> {
    check_hub_url_with(url, ALLOW_PLAIN_LOOPBACK)
}

fn check_hub_url_with(url: &str, allow_plain_loopback: bool) -> Result<(), SyncError> {
    let refuse = |why: &str| Err(SyncError::InvalidHubUrl(why.to_owned()));
    let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else {
        return refuse("the address must start with https://");
    };
    if rest
        .bytes()
        .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
    {
        return refuse("the address contains blanks or control characters");
    }
    if rest.contains(['?', '#']) {
        return refuse("the address must not contain a query or a fragment");
    }
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return refuse("the address needs a host and no user name");
    }
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        let Some((inner, tail)) = bracketed.split_once(']') else {
            return refuse("the address is not valid");
        };
        if !(tail.is_empty() || tail.strip_prefix(':').is_some_and(valid_port)) {
            return refuse("the address is not valid");
        }
        format!("[{inner}]")
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) if valid_port(port) => host.to_owned(),
            Some(_) => return refuse("the port is not valid"),
            None => authority.to_owned(),
        }
    };
    if host.is_empty() || host == "[]" {
        return refuse("the address needs a host");
    }
    if !secure && !allow_plain_loopback {
        return refuse("the address must start with https://");
    }
    if !secure && host != "127.0.0.1" && host != "[::1]" {
        return refuse("plain http is only allowed to 127.0.0.1 and ::1; use https");
    }
    Ok(())
}

fn valid_port(port: &str) -> bool {
    !port.is_empty()
        && port.len() <= 5
        && port.bytes().all(|b| b.is_ascii_digit())
        && port.parse::<u32>().is_ok_and(|p| (1..=65535).contains(&p))
}

/// A configured connection to one hub. Cheap to keep; methods block.
pub struct HubClient {
    /// `None` for [`HubClient::offline`]: every request fails as `hub_unreachable`.
    agent: Option<ureq::Agent>,
    base: String,
    secret: SecretString,
}

impl fmt::Debug for HubClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HubClient")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

fn pinned_roots(pem: &str) -> Result<Vec<Certificate<'static>>, SyncError> {
    let mut certs = Vec::new();
    for item in parse_pem(pem.as_bytes()) {
        if let PemItem::Certificate(c) = item.map_err(|_| SyncError::InvalidCa)? {
            certs.push(c);
        }
    }
    if certs.is_empty() {
        return Err(SyncError::InvalidCa);
    }
    Ok(certs)
}

fn system_roots() -> Vec<Certificate<'static>> {
    rustls_native_certs::load_native_certs()
        .certs
        .iter()
        .map(|der| Certificate::from_der(der.as_ref()).to_owned())
        .collect()
}

impl HubClient {
    pub fn new(cfg: &HubConfig) -> Result<HubClient, SyncError> {
        check_hub_url(&cfg.url)?;
        let https = cfg.url.starts_with("https://");
        let roots = match &cfg.ca_pem {
            Some(pem) => pinned_roots(pem)?,
            None if https => system_roots(),
            None => Vec::new(),
        };
        let tls = TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .root_certs(RootCerts::new_with_certs(&roots))
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(https)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .proxy(None)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(TOTAL_TIMEOUT))
            .build()
            .into();
        Ok(HubClient {
            agent: Some(agent),
            base: cfg.url.trim_end_matches('/').to_owned(),
            secret: cfg.secret.clone(),
        })
    }

    /// A client that never talks: no address, no TLS roots, and every request fails as
    /// `hub_unreachable` without touching the network. For the engine calls that use only the
    /// local side (`Engine::pending_changes`) or build their own client (`setup`, `join`), so no
    /// placeholder address has to pass [`check_hub_url`].
    #[must_use]
    pub fn offline() -> HubClient {
        HubClient {
            agent: None,
            base: String::new(),
            secret: SecretString::from(String::new()),
        }
    }

    fn agent(&self) -> Result<&ureq::Agent, SyncError> {
        self.agent.as_ref().ok_or(SyncError::HubUnreachable)
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.secret.expose_secret())
    }

    /// `GET /v1/info`.
    pub fn info(&self) -> Result<HubInfo, SyncError> {
        let resp = self
            .agent()?
            .get(format!("{}/v1/info", self.base))
            .header("Authorization", self.bearer())
            .call()
            .map_err(|e| map_transport(&e))?;
        match resp.status().as_u16() {
            200 => {}
            other => return Err(status_error(other)),
        }
        let body = read_bounded(resp.into_body().into_reader(), INFO_MAX_LEN)?;
        serde_json::from_slice(&body)
            .map_err(|_| SyncError::HubProtocol("the info answer is not valid".to_owned()))
    }

    /// `GET /v1/head`: `None` when the hub holds nothing yet, otherwise the `seq` and the envelope.
    pub fn get_head(&self) -> Result<Option<(u64, Vec<u8>)>, SyncError> {
        let resp = self
            .agent()?
            .get(format!("{}/v1/head", self.base))
            .header("Authorization", self.bearer())
            .call()
            .map_err(|e| map_transport(&e))?;
        match resp.status().as_u16() {
            200 => {}
            404 => return Ok(None),
            other => return Err(status_error(other)),
        }
        let etag_seq = resp
            .headers()
            .get("ETag")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix('"')?.strip_suffix('"')?.parse::<u64>().ok())
            .ok_or_else(|| SyncError::HubProtocol("the head has no valid ETag".to_owned()))?;
        if let Some(len) = resp
            .headers()
            .get("Content-Length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            && len > MAX_ENVELOPE_LEN as u64
        {
            return Err(SyncError::HubTooLarge);
        }
        let body = read_bounded(resp.into_body().into_reader(), MAX_ENVELOPE_LEN)?;
        match peek_header(&body) {
            Ok((_, seq)) if seq == etag_seq => Ok(Some((seq, body))),
            Ok(_) => Err(SyncError::HubProtocol(
                "the ETag and the envelope disagree about the sequence number".to_owned(),
            )),
            Err(SyncError::NewerFormat { found, supported }) => {
                Err(SyncError::NewerFormat { found, supported })
            }
            Err(_) => Err(SyncError::HubProtocol(
                "the head is not a sync envelope".to_owned(),
            )),
        }
    }

    /// `PUT /v1/head` with a compare-and-swap precondition; a stale one is `hub_conflict`.
    /// `seq` is the number inside `bytes`; the hub reads it from the envelope header.
    pub fn put_head(&self, expected: Expected, seq: u64, bytes: &[u8]) -> Result<(), SyncError> {
        if bytes.len() > MAX_ENVELOPE_LEN {
            return Err(SyncError::HubTooLarge);
        }
        match peek_header(bytes) {
            Ok((_, header_seq)) if header_seq == seq => {}
            _ => {
                return Err(SyncError::HubProtocol(
                    "the data to upload is not an envelope with this sequence number".to_owned(),
                ));
            }
        }
        let request = self
            .agent()?
            .put(format!("{}/v1/head", self.base))
            .header("Authorization", self.bearer())
            .header("Content-Type", "application/octet-stream");
        let request = match expected {
            Expected::Empty => request.header("If-None-Match", "*"),
            Expected::Seq(n) => request.header("If-Match", format!("\"{n}\"")),
        };
        let resp = request.send(bytes).map_err(|e| map_transport(&e))?;
        match resp.status().as_u16() {
            201 | 204 => Ok(()),
            other => Err(status_error(other)),
        }
    }
}

fn status_error(status: u16) -> SyncError {
    match status {
        401 => SyncError::HubUnauthorized,
        412 => SyncError::HubConflict,
        413 => SyncError::HubTooLarge,
        300..=399 => SyncError::HubProtocol("the hub answered with a redirect".to_owned()),
        other => SyncError::HubProtocol(format!("unexpected status {other}")),
    }
}

/// Reads at most `limit` bytes; one more byte is `hub_too_large` and the read stops there.
fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, SyncError> {
    let mut out = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| map_io(&e))?;
    if out.len() > limit {
        return Err(SyncError::HubTooLarge);
    }
    Ok(out)
}

fn map_rustls(e: &rustls::Error) -> SyncError {
    match e {
        rustls::Error::InvalidCertificate(_) | rustls::Error::NoCertificatesPresented => {
            SyncError::HubTlsUntrusted
        }
        _ => SyncError::HubProtocol("the TLS handshake failed".to_owned()),
    }
}

fn map_io(e: &std::io::Error) -> SyncError {
    if let Some(inner) = e.get_ref() {
        if let Some(tls) = inner.downcast_ref::<rustls::Error>() {
            return map_rustls(tls);
        }
        if let Some(wrapped) = inner.downcast_ref::<ureq::Error>() {
            return map_transport(wrapped);
        }
    }
    SyncError::HubUnreachable
}

fn map_transport(e: &ureq::Error) -> SyncError {
    match e {
        ureq::Error::Rustls(tls) => map_rustls(tls),
        ureq::Error::Io(io) => map_io(io),
        ureq::Error::Timeout(_) | ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => {
            SyncError::HubUnreachable
        }
        ureq::Error::BodyExceedsLimit(_) => SyncError::HubTooLarge,
        _ => SyncError::HubProtocol("the request could not be completed".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{GroupId, SyncKey, seal};
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    fn config(url: &str) -> HubConfig {
        HubConfig {
            url: url.to_owned(),
            secret: SecretString::from("sentinel-secret".to_owned()),
            ca_pem: None,
        }
    }

    /// Serves every connection with `handler` (it reads the request head itself).
    fn stub(handler: impl Fn(TcpStream) + Send + Sync + 'static) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handler = Arc::new(handler);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let handler = Arc::clone(&handler);
                thread::spawn(move || handler(stream));
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn read_request_head(stream: &mut TcpStream) {
        let mut seen = Vec::new();
        let mut byte = [0u8; 1];
        while !seen.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).unwrap_or(0) == 0 {
                return;
            }
            seen.push(byte[0]);
        }
    }

    fn envelope(seq: u64) -> Vec<u8> {
        seal(
            &SyncKey::from_bytes([1; 32]),
            GroupId::from_u128(9),
            seq,
            b"{}",
        )
        .unwrap()
    }

    #[test]
    fn plain_http_to_a_non_loopback_host_is_refused() {
        for url in [
            "http://hub.example.test",
            "http://localhost:8787",
            "http://192.168.1.10",
            "http://127.0.0.1.evil.test/",
            "http://user@127.0.0.1/",
            "ftp://x",
            "hub.example.test",
            // a query or fragment would be glued onto every request path
            "https://hub.example.test/?x=1",
            "https://hub.example.test/#frag",
            "https://hub.example.test?x",
            "http://127.0.0.1:8787/?x",
        ] {
            let err = HubClient::new(&config(url)).unwrap_err();
            assert_eq!(err.code(), "invalid_hub_url", "{url}");
        }
        assert!(HubClient::new(&config("https://hub.example.test")).is_ok());
        for url in ["http://127.0.0.1:8787", "http://[::1]:8787/"] {
            assert_eq!(
                HubClient::new(&config(url)).is_ok(),
                ALLOW_PLAIN_LOOPBACK,
                "{url}"
            );
        }
    }

    #[test]
    fn an_offline_client_needs_no_address_and_never_talks() {
        let client = HubClient::offline();
        assert_eq!(client.info().unwrap_err(), SyncError::HubUnreachable);
        assert_eq!(client.get_head().unwrap_err(), SyncError::HubUnreachable);
        let bytes = envelope(1);
        assert_eq!(
            client.put_head(Expected::Empty, 1, &bytes).unwrap_err(),
            SyncError::HubUnreachable
        );
    }

    #[test]
    fn plain_http_to_loopback_is_refused_where_it_is_not_allowed() {
        for url in ["http://127.0.0.1:8787", "http://[::1]:8787/"] {
            let err = check_hub_url_with(url, false).unwrap_err();
            assert_eq!(err.code(), "invalid_hub_url", "{url}");
            check_hub_url_with(url, true).unwrap();
        }
        check_hub_url_with("https://127.0.0.1:8787", false).unwrap();
        // debug builds (the tests) allow it, release builds do not
        assert_eq!(ALLOW_PLAIN_LOOPBACK, cfg!(debug_assertions));
    }

    #[test]
    fn a_redirect_is_refused() {
        let target_hits = Arc::new(AtomicU64::new(0));
        let hits = Arc::clone(&target_hits);
        let target = stub(move |mut s| {
            hits.fetch_add(1, Ordering::SeqCst);
            read_request_head(&mut s);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        });
        let location = format!("{target}/v1/head");
        let origin = stub(move |mut s| {
            read_request_head(&mut s);
            let reply =
                format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n");
            let _ = s.write_all(reply.as_bytes());
        });
        let client = HubClient::new(&config(&origin)).unwrap();
        assert_eq!(client.get_head().unwrap_err().code(), "hub_protocol");
        assert_eq!(
            client
                .put_head(Expected::Empty, 1, &envelope(1))
                .unwrap_err()
                .code(),
            "hub_protocol"
        );
        assert_eq!(
            target_hits.load(Ordering::SeqCst),
            0,
            "the redirect was followed"
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn a_body_above_64_MiB_is_hub_too_large_and_is_not_buffered() {
        // A declared length above the limit is refused before any body byte is read.
        let declared = stub(|mut s| {
            read_request_head(&mut s);
            let _ =
                s.write_all(b"HTTP/1.1 200 OK\r\nETag: \"1\"\r\nContent-Length: 100000000\r\n\r\n");
            thread::sleep(Duration::from_millis(500));
        });
        let err = HubClient::new(&config(&declared))
            .unwrap()
            .get_head()
            .unwrap_err();
        assert_eq!(err.code(), "hub_too_large");

        // No declared length: the stream is cut once the limit is passed.
        let written = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&written);
        let streaming = stub(move |mut s| {
            read_request_head(&mut s);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nETag: \"1\"\r\nConnection: close\r\n\r\n");
            let chunk = vec![0u8; 1024 * 1024];
            for _ in 0..192 {
                if s.write_all(&chunk).is_err() {
                    return;
                }
                counter.fetch_add(chunk.len() as u64, Ordering::SeqCst);
            }
        });
        let err = HubClient::new(&config(&streaming))
            .unwrap()
            .get_head()
            .unwrap_err();
        assert_eq!(err.code(), "hub_too_large");
        thread::sleep(Duration::from_millis(300));
        assert!(
            written.load(Ordering::SeqCst) < 150 * 1024 * 1024,
            "the client kept draining the connection"
        );
    }

    #[test]
    fn a_200_for_head_without_an_etag_is_hub_protocol() {
        let body = envelope(1);
        let url = stub(move |mut s| {
            read_request_head(&mut s);
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&body);
        });
        let err = HubClient::new(&config(&url))
            .unwrap()
            .get_head()
            .unwrap_err();
        assert_eq!(err.code(), "hub_protocol");
    }

    #[test]
    fn a_200_that_is_not_an_envelope_or_disagrees_with_the_etag_is_hub_protocol() {
        let garbage = stub(|mut s| {
            read_request_head(&mut s);
            let _ =
                s.write_all(b"HTTP/1.1 200 OK\r\nETag: \"1\"\r\nContent-Length: 5\r\n\r\nhello");
        });
        assert_eq!(
            HubClient::new(&config(&garbage))
                .unwrap()
                .get_head()
                .unwrap_err()
                .code(),
            "hub_protocol"
        );
        let body = envelope(5);
        let mismatch = stub(move |mut s| {
            read_request_head(&mut s);
            let head = format!(
                "HTTP/1.1 200 OK\r\nETag: \"6\"\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&body);
        });
        assert_eq!(
            HubClient::new(&config(&mismatch))
                .unwrap()
                .get_head()
                .unwrap_err()
                .code(),
            "hub_protocol"
        );
    }

    #[test]
    fn connection_refused_is_hub_unreachable() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let client = HubClient::new(&config(&format!("http://127.0.0.1:{port}"))).unwrap();
        assert_eq!(client.get_head().unwrap_err().code(), "hub_unreachable");
        assert_eq!(client.info().unwrap_err().code(), "hub_unreachable");
    }

    #[test]
    fn a_404_is_none_and_a_401_is_hub_unauthorized() {
        let missing = stub(|mut s| {
            read_request_head(&mut s);
            let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        });
        assert!(
            HubClient::new(&config(&missing))
                .unwrap()
                .get_head()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn a_401_is_hub_unauthorized() {
        let url = stub(|mut s| {
            read_request_head(&mut s);
            let _ = s.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n");
        });
        let client = HubClient::new(&config(&url)).unwrap();
        assert_eq!(client.get_head().unwrap_err().code(), "hub_unauthorized");
        assert_eq!(client.info().unwrap_err().code(), "hub_unauthorized");
        assert_eq!(
            client
                .put_head(Expected::Empty, 1, &envelope(1))
                .unwrap_err()
                .code(),
            "hub_unauthorized"
        );
    }

    #[test]
    fn debug_never_shows_the_secret() {
        let cfg = config("http://127.0.0.1:1");
        assert!(!format!("{cfg:?}").contains("sentinel-secret"));
        let client = HubClient::new(&cfg).unwrap();
        assert!(!format!("{client:?}").contains("sentinel-secret"));
        let err = client.get_head().unwrap_err();
        assert!(!format!("{err} {err:?}").contains("sentinel-secret"));
    }
}
