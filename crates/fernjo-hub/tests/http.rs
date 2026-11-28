//! The hub over a real loopback socket, with a minimal hand-written HTTP/1.1 client.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use fernjo_hub::{HubStore, config, router, serve};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const SECRET: &str = "test-secret";

struct Hub {
    _dir: TempDir,
    addr: SocketAddr,
}

async fn start_with(hash: [u8; 32]) -> Hub {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(HubStore::open(dir.path()).unwrap());
    let addr = serve("127.0.0.1:0".parse().unwrap(), router(store, hash))
        .await
        .unwrap();
    Hub { _dir: dir, addr }
}

async fn start() -> Hub {
    start_with(config::hash_secret(SECRET)).await
}

fn env(seq: u64) -> Vec<u8> {
    let mut v = b"FERNJOSY".to_vec();
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend_from_slice(&[7u8; 16]);
    v.extend_from_slice(&seq.to_be_bytes());
    v.extend_from_slice(&[0xCD; 40]);
    v
}

struct Reply {
    status: u16,
    headers: String,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<String> {
        self.headers.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
        })
    }
}

async fn read_reply(s: &mut TcpStream) -> Reply {
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.ok();
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response head");
    let head = String::from_utf8_lossy(&buf[..split]).into_owned();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let headers = head
        .split_once("\r\n")
        .map(|x| x.1)
        .unwrap_or("")
        .to_owned();
    let mut body = buf[split + 4..].to_vec();
    if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        body = dechunk(&body);
    }
    Reply {
        status,
        headers,
        body,
    }
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(p) = b.windows(2).position(|w| w == b"\r\n") else {
            return out;
        };
        let n = usize::from_str_radix(std::str::from_utf8(&b[..p]).unwrap().trim(), 16).unwrap();
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&b[p + 2..p + 2 + n]);
        b = &b[p + 2 + n + 2..];
    }
}

async fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    extra: &[(&str, &str)],
    body: &[u8],
) -> Reply {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: hub\r\nConnection: close\r\n");
    if let Some(t) = token {
        head.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if method == "PUT" {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    read_reply(&mut s).await
}

#[tokio::test]
async fn no_token_or_wrong_token_is_401() {
    let hub = start().await;
    for path in ["/v1/head", "/v1/info"] {
        let none = request(hub.addr, "GET", path, None, &[], b"").await;
        let wrong = request(hub.addr, "GET", path, Some("nope"), &[], b"").await;
        assert_eq!(none.status, 401);
        assert_eq!(wrong.status, 401);
        assert_eq!(none.body, wrong.body);
    }
    let put = request(
        hub.addr,
        "PUT",
        "/v1/head",
        Some("nope"),
        &[("If-None-Match", "*")],
        &env(1),
    )
    .await;
    assert_eq!(put.status, 401);
}

#[tokio::test]
async fn get_head_of_an_empty_hub_is_404() {
    let hub = start().await;
    let r = request(hub.addr, "GET", "/v1/head", Some(SECRET), &[], b"").await;
    assert_eq!(r.status, 404);
}

#[tokio::test]
async fn put_then_get_returns_the_bytes_and_the_etag() {
    let hub = start().await;
    let first = request(
        hub.addr,
        "PUT",
        "/v1/head",
        Some(SECRET),
        &[("If-None-Match", "*")],
        &env(1),
    )
    .await;
    assert_eq!(first.status, 201);
    let second = request(
        hub.addr,
        "PUT",
        "/v1/head",
        Some(SECRET),
        &[("If-Match", "\"1\"")],
        &env(2),
    )
    .await;
    assert_eq!(second.status, 204);
    let got = request(hub.addr, "GET", "/v1/head", Some(SECRET), &[], b"").await;
    assert_eq!(got.status, 200);
    assert_eq!(got.header("etag").as_deref(), Some("\"2\""));
    assert_eq!(got.body, env(2));
}

#[tokio::test]
async fn put_without_a_precondition_or_with_a_bad_body_is_refused() {
    let hub = start().await;
    let none = request(hub.addr, "PUT", "/v1/head", Some(SECRET), &[], &env(1)).await;
    assert_eq!(none.status, 428);
    let bad = request(
        hub.addr,
        "PUT",
        "/v1/head",
        Some(SECRET),
        &[("If-None-Match", "*")],
        b"not an envelope at all, but long enough to hold a header....",
    )
    .await;
    assert_eq!(bad.status, 400);
    let get = request(hub.addr, "GET", "/v1/head", Some(SECRET), &[], b"").await;
    assert_eq!(get.status, 404);
}

#[tokio::test]
async fn put_with_a_stale_if_match_is_412() {
    let hub = start().await;
    for (m, seq) in [(("If-None-Match", "*"), 1), (("If-Match", "\"1\""), 2)] {
        let r = request(hub.addr, "PUT", "/v1/head", Some(SECRET), &[m], &env(seq)).await;
        assert!(r.status == 201 || r.status == 204);
    }
    let stale = request(
        hub.addr,
        "PUT",
        "/v1/head",
        Some(SECRET),
        &[("If-Match", "\"1\"")],
        &env(3),
    )
    .await;
    assert_eq!(stale.status, 412);
    let again = request(
        hub.addr,
        "PUT",
        "/v1/head",
        Some(SECRET),
        &[("If-None-Match", "*")],
        &env(3),
    )
    .await;
    assert_eq!(again.status, 412);
    let not_higher = request(
        hub.addr,
        "PUT",
        "/v1/head",
        Some(SECRET),
        &[("If-Match", "\"2\"")],
        &env(2),
    )
    .await;
    assert_eq!(not_higher.status, 409);
    let got = request(hub.addr, "GET", "/v1/head", Some(SECRET), &[], b"").await;
    assert_eq!(got.body, env(2));
}

#[tokio::test]
async fn put_over_the_limit_is_413_without_buffering_it_all() {
    let hub = start().await;
    const LIMIT: usize = 64 * 1024 * 1024;

    // 1. announced length: refused from the header alone, the body is never sent.
    let mut s = TcpStream::connect(hub.addr).await.unwrap();
    let head = format!(
        "PUT /v1/head HTTP/1.1\r\nHost: hub\r\nConnection: close\r\nAuthorization: Bearer {SECRET}\r\nIf-None-Match: *\r\nContent-Length: {}\r\n\r\n",
        LIMIT + 1
    );
    s.write_all(head.as_bytes()).await.unwrap();
    let r = read_reply(&mut s).await;
    assert_eq!(r.status, 413);

    let chunked_head = format!(
        "PUT /v1/head HTTP/1.1\r\nHost: hub\r\nConnection: close\r\nAuthorization: Bearer {SECRET}\r\nIf-None-Match: *\r\nTransfer-Encoding: chunked\r\n\r\n"
    );

    // 2. chunked, no length: one byte more than the hub takes (the limit plus its framing room,
    // `BODY_SLACK` in `server.rs`), and the body never ends. The hub has to answer from what it
    // read; one that waited for the end would never answer. Nothing is left unread, so the
    // connection closes cleanly and the reply arrives on every platform.
    let mut s = TcpStream::connect(hub.addr).await.unwrap();
    s.write_all(chunked_head.as_bytes()).await.unwrap();
    let over = LIMIT + 4096 + 1;
    s.write_all(format!("{over:x}\r\n").as_bytes())
        .await
        .unwrap();
    s.write_all(&vec![0u8; over]).await.unwrap();
    let r = tokio::time::timeout(Duration::from_secs(60), read_reply(&mut s))
        .await
        .expect("the hub waited for the end of the body");
    assert_eq!(r.status, 413);

    // 3. chunked and far too large: the hub does not take the rest.
    let mut s = TcpStream::connect(hub.addr).await.unwrap();
    s.write_all(chunked_head.as_bytes()).await.unwrap();
    let chunk = vec![0u8; 1024 * 1024];
    let total_chunks = 200usize; // 200 MiB offered
    let mut sent = 0usize;
    let (mut rd, mut wr) = s.into_split();
    let reader = tokio::spawn(async move {
        let mut buf = Vec::new();
        rd.read_to_end(&mut buf).await.ok();
        buf
    });
    for _ in 0..total_chunks {
        let frame = format!("{:x}\r\n", chunk.len());
        if wr.write_all(frame.as_bytes()).await.is_err()
            || wr.write_all(&chunk).await.is_err()
            || wr.write_all(b"\r\n").await.is_err()
        {
            break;
        }
        sent += chunk.len();
    }
    let _ = wr.shutdown().await;
    let buf = reader.await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    // Closing with unread data resets the connection, and Windows then drops the reply the client
    // has not read yet (step 2 covers the status there).
    if cfg!(not(windows)) {
        assert!(
            text.starts_with("HTTP/1.1 413"),
            "{}",
            &text[..text.len().min(80)]
        );
    }
    assert!(
        sent < total_chunks * chunk.len(),
        "the hub took the whole body"
    );

    let get = request(hub.addr, "GET", "/v1/head", Some(SECRET), &[], b"").await;
    assert_eq!(get.status, 404);
}

#[tokio::test]
async fn info_reports_protocol_1() {
    let hub = start().await;
    let r = request(hub.addr, "GET", "/v1/info", Some(SECRET), &[], b"").await;
    assert_eq!(r.status, 200);
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(v["protocol"], 1);
    assert_eq!(v["maxBytes"], 64 * 1024 * 1024);
}

#[tokio::test]
async fn rotate_secret_invalidates_the_old_one() {
    let dir = TempDir::new().unwrap();
    let old = config::init(dir.path()).unwrap();
    assert!(config::init(dir.path()).is_err());
    let hash_old = config::load_secret_hash(dir.path()).unwrap();
    assert_eq!(hash_old, config::hash_secret(&old));
    let new = config::rotate_secret(dir.path()).unwrap();
    assert_ne!(old, new);
    let hash_new = config::load_secret_hash(dir.path()).unwrap();
    let hub = start_with(hash_new).await;
    let with_old = request(hub.addr, "GET", "/v1/info", Some(&old), &[], b"").await;
    let with_new = request(hub.addr, "GET", "/v1/info", Some(&new), &[], b"").await;
    assert_eq!(with_old.status, 401);
    assert_eq!(with_new.status, 200);
    // serve refuses without init
    let empty = TempDir::new().unwrap();
    assert!(config::load_secret_hash(empty.path()).is_err());
}
