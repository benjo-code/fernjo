use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use super::*;
use crate::interaction::{HostKeySyncedPrompt, KiRound};
use secrecy::SecretString;

const ED_A: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIJ4tyazBiaSQKaes5aSfu9yuLDHNJEPyJiAHqg83VN5T";
const FP_ED_A: &str = "SHA256:fJx8pIbcHzkR2+fd1fj6DszwZ7Uzsw82br7NCdiBVQg";
const ED_B: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIMBS1jUJrjneokA1w91/JopLVVupB9FJxwYnQw8HiGVt";
const EC_X: &str = "AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBKMNL3Oj68qibnhw2CsFjpUuUG0PT24VO3pPh7xdd24NHoWiNtA5zM5HfyvdTlx5GtITgyTX0XRZjFOOd3mpCDg=";
const FP_EC_X: &str = "SHA256:L1oGUojNYTobppCbpYRKbpqCK2TMVx8HPOE4uztJs5k";

fn key(b64: &str) -> PublicKey {
    russh::keys::parse_public_key_base64(b64).unwrap()
}

#[derive(Default)]
struct Script {
    unknown: Mutex<VecDeque<HostKeyDecision>>,
    synced: Mutex<VecDeque<HostKeyDecision>>,
    changed: Mutex<VecDeque<HostKeyChangedDecision>>,
    unknown_prompts: Mutex<Vec<HostKeyPrompt>>,
    synced_prompts: Mutex<Vec<HostKeySyncedPrompt>>,
    changed_prompts: Mutex<Vec<HostKeyChangedPrompt>>,
}

impl Script {
    fn unknown(self, d: HostKeyDecision) -> Self {
        self.unknown.lock().unwrap().push_back(d);
        self
    }
    fn synced(self, d: HostKeyDecision) -> Self {
        self.synced.lock().unwrap().push_back(d);
        self
    }
    fn changed(self, d: HostKeyChangedDecision) -> Self {
        self.changed.lock().unwrap().push_back(d);
        self
    }
    /// Number of (unknown, synced, changed) prompts shown.
    fn prompts(&self) -> (usize, usize, usize) {
        (
            self.unknown_prompts.lock().unwrap().len(),
            self.synced_prompts.lock().unwrap().len(),
            self.changed_prompts.lock().unwrap().len(),
        )
    }
}

impl Interaction for Script {
    async fn host_key(&self, prompt: HostKeyPrompt) -> HostKeyDecision {
        self.unknown_prompts.lock().unwrap().push(prompt);
        self.unknown
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(HostKeyDecision::Reject)
    }
    async fn host_key_synced(&self, prompt: HostKeySyncedPrompt) -> HostKeyDecision {
        self.synced_prompts.lock().unwrap().push(prompt);
        self.synced
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(HostKeyDecision::Reject)
    }
    async fn host_key_changed(&self, prompt: HostKeyChangedPrompt) -> HostKeyChangedDecision {
        self.changed_prompts.lock().unwrap().push(prompt);
        self.changed
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(HostKeyChangedDecision::Reject)
    }
    async fn keyboard_interactive(&self, _round: KiRound) -> Option<Vec<SecretString>> {
        None
    }
    async fn password(&self, _user: &str, _host: &str) -> Option<SecretString> {
        None
    }
    async fn username(&self, _host: &str) -> Option<String> {
        None
    }
    async fn key_passphrase(&self) -> Option<SecretString> {
        None
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    known: PathBuf,
    confirmed: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    Fixture {
        known: dir.path().join("known_hosts"),
        confirmed: dir.path().join("device").join("known_hosts.confirmed"),
        dir,
    }
}

fn handler(
    known: &Path,
    confirmed: Option<KnownHosts>,
    script: Script,
) -> (ClientHandler<Script>, Arc<Script>) {
    let script = Arc::new(script);
    (
        ClientHandler {
            host: "Host.Example".into(),
            port: 2222,
            known_hosts: KnownHosts::new(known),
            confirmed,
            interaction: Arc::clone(&script),
            rejection: Rejection::default(),
            prompting: Arc::new(AtomicBool::new(false)),
            pending_trust: PendingTrust::default(),
        },
        script,
    )
}

fn rejection(h: &ClientHandler<Script>) -> Option<SshError> {
    h.rejection.lock().unwrap().take()
}

fn status(path: &Path, k: &PublicKey) -> HostKeyStatus {
    KnownHosts::new(path)
        .check("host.example", 2222, k)
        .unwrap()
}

fn add(path: &Path, k: &PublicKey) {
    KnownHosts::new(path).add("host.example", 2222, k).unwrap();
}

#[tokio::test]
async fn known_and_confirmed_connects_without_a_prompt() {
    let f = fixture();
    add(&f.known, &key(ED_A));
    add(&f.confirmed, &key(ED_A));
    let (h, script) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default(),
    );
    assert!(h.decide(&key(ED_A)).await);
    assert_eq!(script.prompts(), (0, 0, 0));
}

#[test]
fn the_same_host_spelled_differently_is_one_confirmed_entry() {
    let f = fixture();
    let c = KnownHosts::new(&f.confirmed);
    c.add("HOST.example", 2222, &key(ED_A)).unwrap();
    assert_eq!(
        c.check("host.EXAMPLE", 2222, &key(ED_A)).unwrap(),
        HostKeyStatus::Known
    );
}

#[tokio::test]
async fn known_but_unconfirmed_asks_and_always_records_so_the_next_attempt_is_silent() {
    let f = fixture();
    add(&f.known, &key(ED_A));
    let (h, script) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default().synced(HostKeyDecision::TrustAlways),
    );
    assert!(h.decide(&key(ED_A)).await);
    assert_eq!(script.prompts(), (0, 1, 0));
    let prompt = script.synced_prompts.lock().unwrap()[0].clone();
    assert_eq!(
        (prompt.host.as_str(), prompt.port, prompt.key_type.as_str()),
        ("Host.Example", 2222, "ssh-ed25519")
    );
    assert_eq!(prompt.fingerprint, FP_ED_A);
    assert!(
        !f.confirmed.exists(),
        "nothing is written before the handshake"
    );
    h.apply_pending_trust().await.unwrap();
    assert_eq!(status(&f.confirmed, &key(ED_A)), HostKeyStatus::Known);
    // A fresh handler and an empty script: a prompt would answer Reject.
    let (h, script) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default(),
    );
    assert!(h.decide(&key(ED_A)).await);
    assert_eq!(script.prompts(), (0, 0, 0));
}

#[tokio::test]
async fn once_connects_but_records_nothing() {
    let f = fixture();
    add(&f.known, &key(ED_A));
    for _ in 0..2 {
        let (h, script) = handler(
            &f.known,
            Some(KnownHosts::new(&f.confirmed)),
            Script::default().synced(HostKeyDecision::TrustOnce),
        );
        assert!(h.decide(&key(ED_A)).await);
        assert_eq!(script.prompts(), (0, 1, 0), "asks every time");
        assert!(!f.confirmed.exists());
    }
}

#[tokio::test]
async fn reject_of_a_synced_key_is_host_key_rejected_and_records_nothing() {
    let f = fixture();
    add(&f.known, &key(ED_A));
    let (h, _) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default().synced(HostKeyDecision::Reject),
    );
    assert!(!h.decide(&key(ED_A)).await);
    assert!(matches!(rejection(&h), Some(SshError::HostKeyRejected)));
    assert!(!f.confirmed.exists());
}

#[tokio::test]
async fn a_key_that_differs_from_the_confirmed_one_is_a_changed_key() {
    // `known_hosts` (synced) holds B, this device confirmed A: the warning names A, and
    // "replace" confirms B on this device without touching `known_hosts`.
    let f = fixture();
    add(&f.known, &key(ED_B));
    add(&f.confirmed, &key(ED_A));
    let known_before = std::fs::read(&f.known).unwrap();
    let (h, script) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default().changed(HostKeyChangedDecision::Replace),
    );
    assert!(h.decide(&key(ED_B)).await);
    assert_eq!(script.prompts(), (0, 0, 1));
    let prompt = script.changed_prompts.lock().unwrap()[0].clone();
    assert_eq!(prompt.old, vec![FP_ED_A.to_string()]);
    h.apply_pending_trust().await.unwrap();
    // The confirmed key was replaced, not duplicated.
    assert_eq!(status(&f.confirmed, &key(ED_B)), HostKeyStatus::Known);
    assert!(matches!(
        status(&f.confirmed, &key(ED_A)),
        HostKeyStatus::Changed { .. }
    ));
    assert_eq!(std::fs::read(&f.known).unwrap(), known_before);
}

/// M11 review #24, variant a: whoever can write the synced folder deletes the host's lines from
/// `known_hosts`, and a machine in the path presents a key of another type. This device confirmed
/// an ed25519 key, so the person gets the changed-key warning naming it, not a first-contact
/// prompt.
#[tokio::test]
async fn a_host_this_device_confirmed_is_never_a_first_contact_even_when_known_hosts_lost_it() {
    let f = fixture();
    add(&f.confirmed, &key(ED_A));
    let (h, script) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default(),
    );
    assert!(!h.decide(&key(EC_X)).await);
    assert_eq!(script.prompts(), (0, 0, 1));
    let prompt = script.changed_prompts.lock().unwrap()[0].clone();
    assert_eq!(prompt.old, vec![FP_ED_A.to_string()]);
    assert_eq!(prompt.new, FP_EC_X);
    assert_eq!(prompt.key_type, "ecdsa-sha2-nistp256");
    assert!(matches!(
        rejection(&h),
        Some(SshError::HostKeyChangedRejected)
    ));
}

/// M11 review #24, variant b: the writer adds an ECDSA line for the host to `known_hosts`. The key
/// is "known" there but this device confirmed only its ed25519 key: the changed-key warning, with
/// the confirmed fingerprint, not the mild "came from sync" prompt.
#[tokio::test]
async fn a_key_of_another_type_planted_in_known_hosts_is_a_changed_key() {
    let f = fixture();
    add(&f.known, &key(EC_X));
    add(&f.known, &key(ED_A));
    add(&f.confirmed, &key(ED_A));
    let (h, script) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default().changed(HostKeyChangedDecision::Replace),
    );
    assert!(h.decide(&key(EC_X)).await);
    assert_eq!(script.prompts(), (0, 0, 1));
    let prompt = script.changed_prompts.lock().unwrap()[0].clone();
    assert_eq!(prompt.old, vec![FP_ED_A.to_string()]);
    // "replace" confirms the new key next to the old one of the other type
    h.apply_pending_trust().await.unwrap();
    assert_eq!(status(&f.confirmed, &key(EC_X)), HostKeyStatus::Known);
    assert_eq!(status(&f.confirmed, &key(ED_A)), HostKeyStatus::Known);
}

#[tokio::test]
async fn unknown_host_trust_always_writes_both_files() {
    let f = fixture();
    let (h, script) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default().unknown(HostKeyDecision::TrustAlways),
    );
    assert!(h.decide(&key(ED_A)).await);
    assert_eq!(script.prompts(), (1, 0, 0));
    h.apply_pending_trust().await.unwrap();
    assert_eq!(status(&f.known, &key(ED_A)), HostKeyStatus::Known);
    assert_eq!(status(&f.confirmed, &key(ED_A)), HostKeyStatus::Known);
}

#[tokio::test]
async fn unknown_host_trust_once_writes_neither_file() {
    let f = fixture();
    let (h, _) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default().unknown(HostKeyDecision::TrustOnce),
    );
    assert!(h.decide(&key(ED_A)).await);
    assert!(!f.known.exists() && !f.confirmed.exists());
}

#[tokio::test]
async fn changed_host_replace_updates_both_files() {
    let f = fixture();
    add(&f.known, &key(ED_A));
    add(&f.confirmed, &key(ED_A));
    let (h, script) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default().changed(HostKeyChangedDecision::Replace),
    );
    assert!(h.decide(&key(ED_B)).await);
    assert_eq!(script.prompts(), (0, 0, 1));
    h.apply_pending_trust().await.unwrap();
    for path in [&f.known, &f.confirmed] {
        assert_eq!(status(path, &key(ED_B)), HostKeyStatus::Known);
        assert!(matches!(
            status(path, &key(ED_A)),
            HostKeyStatus::Changed { .. }
        ));
    }
}

#[tokio::test]
async fn without_a_confirmed_layer_behaviour_is_as_before() {
    let f = fixture();
    add(&f.known, &key(ED_A));
    let (h, script) = handler(&f.known, None, Script::default());
    assert!(h.decide(&key(ED_A)).await);
    assert_eq!(script.prompts(), (0, 0, 0));
    let (h, script) = handler(
        &f.known,
        None,
        Script::default().changed(HostKeyChangedDecision::Replace),
    );
    assert!(h.decide(&key(ED_B)).await);
    assert_eq!(script.prompts(), (0, 0, 1));
    h.apply_pending_trust().await.unwrap();
    assert_eq!(status(&f.known, &key(ED_B)), HostKeyStatus::Known);
    assert!(!f.dir.path().join("device").exists());
}

#[tokio::test]
async fn an_unwritable_confirmed_file_does_not_stop_the_connect() {
    let f = fixture();
    add(&f.known, &key(ED_A));
    // The folder of the confirmed file is a regular file: it can be neither read nor made.
    let blocker = f.dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let confirmed = KnownHosts::new(blocker.join("known_hosts.confirmed"));
    let (h, script) = handler(
        &f.known,
        Some(confirmed.clone()),
        Script::default().synced(HostKeyDecision::TrustAlways),
    );
    assert!(h.decide(&key(ED_A)).await);
    assert_eq!(script.prompts(), (0, 1, 0));
    assert!(rejection(&h).is_none());
    h.apply_pending_trust().await.unwrap();
    // Changed host, replace: known_hosts is written, the confirmed failure is ignored.
    let (h, _) = handler(
        &f.known,
        Some(confirmed),
        Script::default().changed(HostKeyChangedDecision::Replace),
    );
    assert!(h.decide(&key(ED_B)).await);
    h.apply_pending_trust().await.unwrap();
    assert_eq!(status(&f.known, &key(ED_B)), HostKeyStatus::Known);
}

/// A `known_hosts` path that reads as "no file" (so the decision itself works) but cannot be
/// written. Windows: a path below a regular file reads as not found. Unix: that is
/// `NotADirectory` and would stop the decision, so the folder is read-only instead; `None`
/// when the process can write there anyway (root).
fn unwritable_known_hosts(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let ro = dir.join("readonly");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::write(ro.join("probe"), b"x").is_ok() {
            return None;
        }
        Some(ro.join("known_hosts"))
    }
    #[cfg(not(unix))]
    {
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        Some(blocker.join("known_hosts"))
    }
}

#[tokio::test]
async fn an_unwritable_known_hosts_fails_when_the_trust_is_applied() {
    let f = fixture();
    let Some(path) = unwritable_known_hosts(f.dir.path()) else {
        return; // running as root: a read-only folder does not stop the write
    };
    let (h, _) = handler(
        &path,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default().unknown(HostKeyDecision::TrustAlways),
    );
    assert!(h.decide(&key(ED_A)).await);
    assert!(h.apply_pending_trust().await.is_err());
    assert!(
        !f.confirmed.exists(),
        "nothing is confirmed that was not saved"
    );
}

#[tokio::test]
async fn an_in_memory_confirmed_layer_asks_once_per_process() {
    let f = fixture();
    add(&f.known, &key(ED_A));
    let memory = KnownHosts::in_memory();
    let (h, script) = handler(
        &f.known,
        Some(memory.clone()),
        Script::default().synced(HostKeyDecision::TrustAlways),
    );
    assert!(h.decide(&key(ED_A)).await);
    assert_eq!(script.prompts(), (0, 1, 0));
    h.apply_pending_trust().await.unwrap();
    let (h, script) = handler(&f.known, Some(memory), Script::default());
    assert!(h.decide(&key(ED_A)).await);
    assert_eq!(script.prompts(), (0, 0, 0));
    assert!(!f.confirmed.exists());
}

#[tokio::test]
async fn trust_always_writes_known_hosts_only_after_the_handshake() {
    let f = fixture();
    let (h, _) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default().unknown(HostKeyDecision::TrustAlways),
    );
    assert!(h.decide(&key(ED_A)).await);
    assert!(
        !f.known.exists() && !f.confirmed.exists(),
        "the decision alone writes nothing"
    );
    h.apply_pending_trust().await.unwrap();
    assert_eq!(status(&f.known, &key(ED_A)), HostKeyStatus::Known);
    assert_eq!(status(&f.confirmed, &key(ED_A)), HostKeyStatus::Known);
    // applied once: a second call has nothing left to write
    std::fs::remove_file(&f.known).unwrap();
    h.apply_pending_trust().await.unwrap();
    assert!(!f.known.exists());
}

#[tokio::test]
async fn a_failed_handshake_leaves_known_hosts_unchanged() {
    // Unknown host, trust always.
    let f = fixture();
    let (h, _) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default().unknown(HostKeyDecision::TrustAlways),
    );
    assert!(h.decide(&key(ED_A)).await);
    drop(h); // the handshake failed: apply_pending_trust is never called
    assert!(!f.known.exists() && !f.confirmed.exists());

    // Changed host, replace: both files keep the old key.
    let f = fixture();
    add(&f.known, &key(ED_A));
    add(&f.confirmed, &key(ED_A));
    let before = (
        std::fs::read(&f.known).unwrap(),
        std::fs::read(&f.confirmed).unwrap(),
    );
    let (h, _) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default().changed(HostKeyChangedDecision::Replace),
    );
    assert!(h.decide(&key(ED_B)).await);
    drop(h);
    assert_eq!(
        before,
        (
            std::fs::read(&f.known).unwrap(),
            std::fs::read(&f.confirmed).unwrap()
        )
    );

    // Synced key, trust always: the device does not confirm it.
    let f = fixture();
    add(&f.known, &key(ED_A));
    let (h, _) = handler(
        &f.known,
        Some(KnownHosts::new(&f.confirmed)),
        Script::default().synced(HostKeyDecision::TrustAlways),
    );
    assert!(h.decide(&key(ED_A)).await);
    drop(h);
    assert!(!f.confirmed.exists());
}
