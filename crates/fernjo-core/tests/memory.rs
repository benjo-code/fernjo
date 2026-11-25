//! Peak memory while loading hostile `connections.json` and `settings.json` files (M11 review
//! #28): a document must never be built as a `serde_json::Value` (about 98 bytes per input byte
//! for `[{"":0},…]`) before its shape is checked. A counting global allocator measures the peak
//! of bytes held during each load; the tests run one at a time (`LOCK`) so they do not count
//! each other's allocations.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Mutex, PoisonError};

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
/// Peak memory allowed per input byte. Loading a file reads it once (1x); building a `Value`
/// cost about 98x.
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

fn write(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

const ID: &str = "6f0c1c2a-0000-4000-8000-000000000001";

#[test]
fn connections_json_with_junk_loads_without_building_a_value() {
    let _guard = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let junk = junk(JUNK_BYTES);
    let cases = [
        (
            "unknown top-level key",
            format!(r#"{{"schemaVersion":3,"children":[],"x":{junk}}}"#),
        ),
        (
            "unknown key in a node",
            format!(
                r#"{{"schemaVersion":3,"children":[{{"id":"{ID}","name":"n","type":"folder","x":{junk}}}]}}"#
            ),
        ),
        (
            "unknown key in rdpResolution",
            format!(
                r#"{{"schemaVersion":3,"children":[{{"id":"{ID}","name":"n","type":"connection","protocol":"rdp","settings":{{"rdpResolution":{{"mode":"fit","x":{junk}}}}}}}]}}"#
            ),
        ),
    ];
    let mut over = Vec::new();
    for (what, text) in cases {
        let path = write(dir.path(), fernjo_core::FILE_NAME, &text);
        let (loaded, peak) = peak_during(|| fernjo_core::load(&path));
        let loaded = loaded.unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(loaded.notice.is_none(), "{what}");
        check(what, text.len(), peak, &mut over);
    }
    assert!(over.is_empty(), "above {MAX_RATIO}x: {over:?}");
}

#[test]
fn a_junk_connections_json_is_refused_without_building_a_value() {
    let _guard = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let text = junk(JUNK_BYTES);
    let path = write(dir.path(), fernjo_core::FILE_NAME, &text);
    let (loaded, peak) = peak_during(|| fernjo_core::load(&path));
    assert!(loaded.is_err());
    let mut over = Vec::new();
    check("bare junk array", text.len(), peak, &mut over);
    assert!(over.is_empty(), "above {MAX_RATIO}x: {over:?}");
}

#[test]
fn settings_json_with_junk_loads_without_building_a_value() {
    let _guard = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let junk = junk(JUNK_BYTES);
    let cases = [
        (
            "unknown settings key",
            format!(r#"{{"schemaVersion":1,"x":{junk}}}"#),
        ),
        (
            "junk theme value",
            format!(r#"{{"schemaVersion":1,"theme":{junk}}}"#),
        ),
    ];
    let mut over = Vec::new();
    for (what, text) in cases {
        let path = write(dir.path(), fernjo_core::settings::FILE_NAME, &text);
        let (loaded, peak) = peak_during(|| fernjo_core::settings::load(&path));
        let loaded = loaded.unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(loaded, fernjo_core::AppSettings::default(), "{what}");
        check(what, text.len(), peak, &mut over);
    }
    assert!(over.is_empty(), "above {MAX_RATIO}x: {over:?}");
}
