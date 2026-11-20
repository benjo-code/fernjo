//! THROWAWAY (branch `spike/m08-rdp-canvas`, never merged): measures what the RDP side delivers.
//!
//! Connects with `fernjo_rdp::Session` to the host named by `FERNJO_TEST_RDP_ADDR` (`host:port`),
//! `FERNJO_TEST_RDP_USER`, `FERNJO_TEST_RDP_PASSWORD` (and optionally `FERNJO_TEST_RDP_DOMAIN`),
//! then runs a scripted load against the remote desktop while it counts what comes back:
//!
//! 1. scroll: Win+R, `notepad`, then a line of text and Enter 30 times a second,
//! 2. window moves: Win+Left, Win+Right, Win+Up in turns (the window animates and repaints),
//! 3. repaint: Win+D toggles the desktop.
//!
//! Each phase lasts `FERNJO_BENCH_SECONDS` (default 10). It prints, per phase, the Damage events
//! per second at the consumer (the queue merges while the consumer is behind, so this is the
//! number of frames a UI would have to paint at most), the mean and p95 size of such a message in
//! bytes (rectangle area * 4), the megabytes per second, and the mean time `copy_region` needs to
//! take the pixels out of the framebuffer (the Rust-side cost of one message).
//!
//! Run: `cargo run -p fernjo-rdp --release --example spike_bench`. The load is the same on every
//! machine; judging the typing latency is done by looking at the result in a real client.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fernjo_rdp::{
    CertChangedDecision, CertChangedPrompt, CertDecision, CertPrompt, ConnectParams, InputEvent,
    Interaction, Session, SessionEvent,
};
use secrecy::SecretString;

struct Trust;

impl Interaction for Trust {
    async fn certificate(&self, _prompt: CertPrompt) -> CertDecision {
        CertDecision::TrustOnce
    }
    async fn certificate_changed(&self, _prompt: CertChangedPrompt) -> CertChangedDecision {
        CertChangedDecision::Replace
    }
    async fn username(&self, _host: &str) -> Option<String> {
        None
    }
    async fn password(&self, _user: &str, _host: &str) -> Option<SecretString> {
        None
    }
}

fn key(scancode: u8, extended: bool) -> [InputEvent; 2] {
    [
        InputEvent::Key {
            scancode,
            extended,
            pressed: true,
        },
        InputEvent::Key {
            scancode,
            extended,
            pressed: false,
        },
    ]
}

/// Win + `scancode`.
fn win(scancode: u8, extended: bool) -> Vec<InputEvent> {
    let mut v = vec![InputEvent::Key {
        scancode: 0x5b,
        extended: true,
        pressed: true,
    }];
    v.extend(key(scancode, extended));
    v.push(InputEvent::Key {
        scancode: 0x5b,
        extended: true,
        pressed: false,
    });
    v
}

fn typed(word: &str) -> Vec<InputEvent> {
    let mut v = Vec::new();
    for c in word.chars() {
        let code = match c {
            'a' => 0x1e,
            'd' => 0x20,
            'e' => 0x12,
            'n' => 0x31,
            'o' => 0x18,
            'p' => 0x19,
            't' => 0x14,
            _ => continue,
        };
        v.extend(key(code, false));
    }
    v
}

#[derive(Default)]
struct Phase {
    name: &'static str,
    events: u64,
    bytes: Vec<u64>,
    copy: Duration,
}

impl Phase {
    fn report(&mut self, seconds: u64) {
        self.bytes.sort_unstable();
        let total: u64 = self.bytes.iter().sum();
        let n = self.bytes.len().max(1) as u64;
        let p95 = self
            .bytes
            .get((self.bytes.len() * 95 / 100).min(self.bytes.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0);
        println!(
            "{:<8} {:>6.1} msg/s  mean {:>9} B  p95 {:>9} B  {:>6.1} MB/s  copy {:>6.2} ms/msg",
            self.name,
            self.events as f64 / seconds as f64,
            total / n,
            p95,
            total as f64 / seconds as f64 / 1_048_576.0,
            self.copy.as_secs_f64() * 1000.0 / n as f64,
        );
    }
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("FERNJO_TEST_RDP_ADDR").expect("FERNJO_TEST_RDP_ADDR=host:port");
    let (host, port) = addr.rsplit_once(':').unwrap_or((addr.as_str(), "3389"));
    let mut params = ConnectParams::new(
        host,
        port.parse().expect("a port"),
        std::env::var("FERNJO_TEST_RDP_USER").expect("FERNJO_TEST_RDP_USER"),
        SecretString::from(
            std::env::var("FERNJO_TEST_RDP_PASSWORD").expect("FERNJO_TEST_RDP_PASSWORD"),
        ),
    );
    params.domain = std::env::var("FERNJO_TEST_RDP_DOMAIN").ok();
    params.width = 1920;
    params.height = 1080;
    let seconds: u64 = std::env::var("FERNJO_BENCH_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let (session, mut events) = Session::connect(params, Arc::new(Trust))
        .await
        .expect("connect");
    let session = Arc::new(session);
    let (w, h) = session.framebuffer().size();
    println!("connected, desktop {w}x{h}, {seconds} s per phase");
    // let the first full paint settle
    tokio::time::sleep(Duration::from_secs(2)).await;
    while events.try_recv().is_ok() {}

    for (index, name) in ["scroll", "moves", "repaint"].into_iter().enumerate() {
        let load = {
            let session = session.clone();
            tokio::spawn(async move {
                let end = Instant::now() + Duration::from_secs(seconds);
                if index == 0 {
                    let _ = session.input(win(0x13, false)).await; // Win+R
                    tokio::time::sleep(Duration::from_millis(800)).await;
                    let _ = session.input(typed("notepad")).await;
                    let _ = session.input(key(0x1c, false).to_vec()).await;
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                }
                let mut step = 0usize;
                while Instant::now() < end {
                    let events = match index {
                        0 => {
                            let mut v = typed("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
                            v.extend(key(0x1c, false));
                            v
                        }
                        1 => match step % 3 {
                            0 => win(0x4b, true),
                            1 => win(0x4d, true),
                            _ => win(0x48, true),
                        },
                        _ => win(0x20, false),
                    };
                    if session.input(events).await.is_err() {
                        break;
                    }
                    step += 1;
                    let pause = match index {
                        0 => 33,
                        1 => 400,
                        _ => 800,
                    };
                    tokio::time::sleep(Duration::from_millis(pause)).await;
                }
            })
        };
        let mut phase = Phase {
            name,
            ..Phase::default()
        };
        let end = Instant::now() + Duration::from_secs(seconds);
        let mut shown = seconds;
        while Instant::now() < end {
            let left = end.saturating_duration_since(Instant::now()).as_secs() + 1;
            if left != shown {
                shown = left;
                eprint!("\r{name}: {left} s left   ");
            }
            if let Ok(Some(SessionEvent::Damage(rect))) =
                tokio::time::timeout(Duration::from_millis(100), events.recv()).await
            {
                let t = Instant::now();
                let pixels = session.framebuffer().copy_region(rect);
                phase.copy += t.elapsed();
                phase.events += 1;
                phase.bytes.push(pixels.len() as u64 + 8);
            }
        }
        eprintln!();
        let _ = load.await;
        phase.report(seconds);
    }
    session.close();
}
