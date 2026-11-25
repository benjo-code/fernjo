//! Peak memory while reading hostile sync snapshots and device state (M11 review #28): a
//! document must never be built as a `serde_json::Value` (about 98 bytes per input byte for
//! `[{"":0},…]`) before its shape is checked. A counting global allocator measures the peak of
//! bytes held during each call; the tests run one at a time (`LOCK`) so they do not count each
//! other's allocations.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Mutex, PoisonError};

use fernjo_sync::snapshot::Snapshot;
use fernjo_sync::state::{BASE_FILE, StateStore};

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
/// Peak memory allowed per input byte. Reading a state file reads it once (1x); building a
/// `Value` cost about 98x.
const MAX_RATIO: usize = 2;

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

const SNAPSHOT_HEAD: &str = r#""formatVersion":1,"groupId":"6f0c1c2a-0000-4000-8000-000000000009","seq":3,"writer":{"id":"6f0c1c2a-0000-4000-8000-000000000005","name":"laptop"},"writtenAt":"2026-11-25""#;

fn snapshot_cases(junk: &str) -> [(&'static str, String); 3] {
    [
        (
            "unknown snapshot key",
            format!(
                r#"{{{SNAPSHOT_HEAD},"connections":{{"schemaVersion":3,"children":[]}},"x":{junk}}}"#
            ),
        ),
        (
            "unknown key in the connections document",
            format!(
                r#"{{{SNAPSHOT_HEAD},"connections":{{"schemaVersion":3,"children":[],"x":{junk}}}}}"#
            ),
        ),
        (
            "unknown key in a node",
            format!(
                r#"{{{SNAPSHOT_HEAD},"connections":{{"schemaVersion":3,"children":[{{"id":"6f0c1c2a-0000-4000-8000-000000000001","name":"n","type":"folder","x":{junk}}}]}}}}"#
            ),
        ),
    ]
}

#[test]
fn a_snapshot_with_junk_is_read_without_building_a_value() {
    let _guard = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let junk = junk(JUNK_BYTES);
    let mut over = Vec::new();
    for (what, text) in snapshot_cases(&junk) {
        let (snapshot, peak) = peak_during(|| Snapshot::from_plaintext(text.as_bytes()));
        snapshot.unwrap_or_else(|e| panic!("{what}: {e}"));
        check(what, text.len(), peak, &mut over);
    }
    let text = junk;
    let (snapshot, peak) = peak_during(|| Snapshot::from_plaintext(text.as_bytes()));
    assert_eq!(snapshot.unwrap_err().code(), "snapshot_invalid");
    check("bare junk array", text.len(), peak, &mut over);
    assert!(over.is_empty(), "above {MAX_RATIO}x: {over:?}");
}

#[test]
fn a_base_snapshot_with_junk_is_read_without_building_a_value() {
    let _guard = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::new(dir.path().to_path_buf());
    let junk = junk(JUNK_BYTES);
    let mut over = Vec::new();
    let mut cases = vec![(
        "unknown key in sync-base.json",
        format!(
            r#"{{"schema":1,"snapshot":{{{SNAPSHOT_HEAD},"connections":{{"schemaVersion":3,"children":[]}}}},"x":{junk}}}"#
        ),
    )];
    for (what, snapshot) in snapshot_cases(&junk) {
        cases.push((what, format!(r#"{{"schema":1,"snapshot":{snapshot}}}"#)));
    }
    for (what, text) in cases {
        std::fs::write(dir.path().join(BASE_FILE), &text).unwrap();
        let (base, peak) = peak_during(|| store.load_base());
        assert!(base.unwrap_or_else(|e| panic!("{what}: {e}")).is_some());
        check(what, text.len(), peak, &mut over);
    }
    assert!(over.is_empty(), "above {MAX_RATIO}x: {over:?}");
}
