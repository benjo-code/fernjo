//! A tiny plain-HTTP server on 127.0.0.1 that plays the update feed and the download host, and
//! logs every request it receives (method, path, query and headers).
//! One thread per connection, `Connection: close`, no keep-alive.

use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct Logged {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    /// Header names are lowercased.
    pub headers: Vec<(String, String)>,
}

pub enum Route {
    /// 200 with a JSON body.
    Json(String),
    /// 200 with the bytes and a matching `Content-Length`.
    Body(Vec<u8>),
    /// 302 to another path on this server.
    Redirect(String),
    /// Announces `declared` bytes, sends `first`, then sends nothing more until the server stops.
    Stall { declared: u64, first: Vec<u8> },
    /// No `Content-Length` (chunked); sends zero bytes in 1 KiB chunks, `total` in all.
    /// `sent` counts the body bytes written; `done` is set when the handler ends (the client went
    /// away, an error, or all `total` bytes sent).
    Endless {
        total: usize,
        sent: Arc<AtomicUsize>,
        done: Arc<AtomicBool>,
    },
    /// A status line only, no body.
    Status(u16),
}

pub struct FakeFeed {
    pub addr: SocketAddr,
    log: Arc<Mutex<Vec<Logged>>>,
    stop: Arc<AtomicBool>,
}

impl FakeFeed {
    pub fn start(routes: Vec<(&'static str, Route)>) -> Self {
        Self::start_with(|_| routes)
    }

    /// Like `start`, but the routes may depend on the address (a manifest holds the download URL).
    pub fn start_with(routes: impl FnOnce(SocketAddr) -> Vec<(&'static str, Route)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let routes = routes(addr);
        let routes: Arc<Vec<(String, Route)>> =
            Arc::new(routes.into_iter().map(|(p, r)| (p.to_owned(), r)).collect());
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (routes, log, stop) = (routes.clone(), log.clone(), stop.clone());
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let (routes, log, stop) = (routes.clone(), log.clone(), stop.clone());
                    thread::spawn(move || serve(stream, &routes, &log, &stop));
                }
            });
        }
        Self { addr, log, stop }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    pub fn log(&self) -> Vec<Logged> {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for FakeFeed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // wake the accept loop so it can see the flag
        let _ = TcpStream::connect(self.addr);
    }
}

fn serve(
    stream: TcpStream,
    routes: &[(String, Route)],
    log: &Mutex<Vec<Logged>>,
    stop: &AtomicBool,
) {
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let target = parts.next().unwrap_or("").to_owned();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
        }
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_owned(), Some(q.to_owned())),
        None => (target.clone(), None),
    };
    log.lock().unwrap().push(Logged {
        method,
        path: path.clone(),
        query,
        headers,
    });
    let Some((_, route)) = routes.iter().find(|(p, _)| *p == path) else {
        let _ = out
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    };
    let _ = respond(&mut out, route, stop);
}

fn respond(out: &mut TcpStream, route: &Route, stop: &AtomicBool) -> std::io::Result<()> {
    match route {
        Route::Json(body) => {
            write!(
                out,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )?;
        }
        Route::Body(bytes) => {
            write!(
                out,
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )?;
            out.write_all(bytes)?;
        }
        Route::Redirect(to) => {
            write!(
                out,
                "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
        Route::Status(code) => {
            write!(
                out,
                "HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
        Route::Stall { declared, first } => {
            write!(
                out,
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
            )?;
            out.write_all(first)?;
            out.flush()?;
            let until = Instant::now() + Duration::from_secs(30);
            while !stop.load(Ordering::SeqCst) && Instant::now() < until {
                thread::sleep(Duration::from_millis(50));
            }
        }
        Route::Endless { total, sent, done } => {
            let result = stream_endless(out, *total, sent);
            done.store(true, Ordering::SeqCst);
            result?;
        }
    }
    out.flush()
}

const CRLF: &[u8] = b"\r\n";
/// 1024 bytes in hex.
const CHUNK_SIZE_LINE: &[u8] = b"400\r\n";
const HEAD_CHUNKED: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";

fn stream_endless(out: &mut TcpStream, total: usize, sent: &AtomicUsize) -> std::io::Result<()> {
    out.write_all(HEAD_CHUNKED)?;
    let chunk = [0u8; 1024];
    let mut written = 0;
    while written < total {
        out.write_all(CHUNK_SIZE_LINE)?;
        out.write_all(&chunk)?;
        out.write_all(CRLF)?;
        out.flush()?;
        written += chunk.len();
        sent.store(written, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(1));
    }
    out.write_all(b"0\r\n\r\n")
}
