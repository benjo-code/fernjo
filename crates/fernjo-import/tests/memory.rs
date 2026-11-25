//! Peak memory while reading hostile connections exports and backups (M11 review #28): a
//! document must never be built as a `serde_json::Value` (about 98 bytes per input byte for
//! `[{"":0},…]`) before its shape is checked. A counting global allocator measures the peak of
//! bytes held during each call; the tests run one at a time (`LOCK`) so they do not count each
//! other's allocations.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Mutex, PoisonError};

use fernjo_import::backup;
use fernjo_import::connections::{self, ConnectionsError};
use fernjo_vault::{KdfParams, seal_with_password};
use secrecy::SecretString;

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grow(by: usize) {
    let now = CURRENT.fetch_add(by, SeqCst) + by;
    PEAK.fetch_max(now, SeqCst);
}

// SAFETY: every call is forwarded to `System`; only the byte counters are added.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grow(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        CURRENT.fetch_sub(layout.size(), SeqCst);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, layout, new_size) };
        if !q.is_null() {
            if new_size > layout.size() {
                grow(new_size - layout.size());
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, SeqCst);
            }
        }
        q
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

static LOCK: Mutex<()> = Mutex::new(());

/// Peak bytes held above the level at the start while `f` runs.
fn peak_during<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = CURRENT.load(SeqCst);
    PEAK.store(base, SeqCst);
    let out = f();
    (out, PEAK.load(SeqCst).saturating_sub(base))
}

/// Input size of every hostile file; small enough for CI, large enough to dwarf fixed costs.
const JUNK_BYTES: usize = 4 * 1024 * 1024;
/// Peak memory allowed per input byte. Opening a backup decrypts it into a second buffer, so the
/// bound is 4x (the review's target); building a `Value` cost about 98x.
const MAX_RATIO: usize = 4;

/// `[{"":0},{"":0},…]` of about `bytes` bytes: each one-key object cost a whole map as a `Value`.
fn junk(bytes: usize) -> String {
    let unit = r#"{"":0}"#;
    let mut s = String::with_capacity(bytes + 8);
    s.push('[');
    while s.len() + unit.len() + 2 < bytes {
        if s.len() > 1 {
            s.push(',');
        }
        s.push_str(unit);
    }
    s.push(']');
    s
}

/// Prints the ratio and records `what` in `over` when the peak is above the bound, so one run
/// shows every case.
fn check(what: &str, input: usize, peak: usize, over: &mut Vec<String>) {
    let ratio = peak as f64 / input as f64;
    eprintln!("{what}: input {input} bytes, peak {peak} bytes ({ratio:.2}x)");
    if peak >= input * MAX_RATIO {
        over.push(format!("{what}: {ratio:.2}x"));
    }
}

const ID: &str = "6f0c1c2a-0000-4000-8000-000000000001";
const TODAY: &str = "2026-11-25";

#[test]
fn a_junk_import_file_is_refused_without_building_a_value() {
    let _guard = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let text = junk(JUNK_BYTES);
    let (plan, peak) = peak_during(|| connections::plan(text.as_bytes(), TODAY, &[]));
    assert_eq!(plan.unwrap_err(), ConnectionsError::NotThisFormat);
    let mut over = Vec::new();
    check("bare junk array", text.len(), peak, &mut over);
    assert!(over.is_empty(), "above {MAX_RATIO}x: {over:?}");
}

#[test]
fn an_import_file_with_junk_is_read_without_building_a_value() {
    let _guard = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let junk = junk(JUNK_BYTES);
    let head = r#""format":"fernjo-connections","schemaVersion":3"#;
    let cases = [
        (
            "unknown top-level key",
            format!(r#"{{{head},"children":[],"x":{junk}}}"#),
        ),
        (
            "unknown key in a node",
            format!(
                r#"{{{head},"children":[{{"id":"{ID}","name":"n","type":"folder","x":{junk}}}]}}"#
            ),
        ),
    ];
    let mut over = Vec::new();
    for (what, text) in cases {
        let (plan, peak) = peak_during(|| connections::plan(text.as_bytes(), TODAY, &[]));
        plan.unwrap_or_else(|e| panic!("{what}: {e}"));
        check(what, text.len(), peak, &mut over);
    }
    assert!(over.is_empty(), "above {MAX_RATIO}x: {over:?}");
}

#[test]
fn a_backup_with_junk_is_opened_without_building_a_value() {
    let _guard = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let junk = junk(JUNK_BYTES);
    let password = SecretString::from("correct horse".to_owned());
    let entry = r#"{"id":"6f0c1c2a-0000-4000-8000-000000000002","name":"e","secret":{"type":"password","password":"p"}}"#;
    let cases = [
        (
            "unknown body key",
            format!(
                r#"{{"schemaVersion":1,"createdAt":"x","app":"y","connections":{{"schemaVersion":3,"children":[]}},"entries":[{entry}],"x":{junk}}}"#
            ),
        ),
        (
            "unknown key in the connections document",
            format!(
                r#"{{"schemaVersion":1,"createdAt":"x","app":"y","connections":{{"schemaVersion":3,"children":[],"x":{junk}}},"entries":[{entry}]}}"#
            ),
        ),
        (
            "unknown key in an entry's secret",
            format!(
                r#"{{"schemaVersion":1,"createdAt":"x","app":"y","connections":{{"schemaVersion":3,"children":[]}},"entries":[{{"id":"6f0c1c2a-0000-4000-8000-000000000002","name":"e","secret":{{"type":"password","password":"p","x":{junk}}}}}]}}"#
            ),
        ),
    ];
    let mut over = Vec::new();
    for (what, body) in cases {
        let sealed = seal_with_password(
            backup::MAGIC,
            backup::VERSION,
            &password,
            KdfParams::TEST,
            body.as_bytes(),
        )
        .unwrap();
        let (opened, peak) = peak_during(|| backup::open(&sealed, &password));
        let opened = opened.unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(opened.entries.len(), 1, "{what}");
        check(what, sealed.len(), peak, &mut over);
    }
    assert!(over.is_empty(), "above {MAX_RATIO}x: {over:?}");
}
