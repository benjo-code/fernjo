//! Shared helpers for the integration tests: the OpenSSH test server and a scripted `Interaction`.
//! Start the server with `eval "$(tools/ssh-test-server.sh start)"` (see that script).
#![allow(dead_code)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

use fernjo_ssh::{
    HostKeyChangedDecision, HostKeyChangedPrompt, HostKeyDecision, HostKeyPrompt, Interaction,
    KiRound,
};
use secrecy::SecretString;

/// Connection data of the test server, read from the environment.
pub struct TestServer {
    pub addr: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub key_ed25519: PathBuf,
    pub key_rsa: PathBuf,
    pub key_encrypted: PathBuf,
    pub key_passphrase: String,
    pub key_unauthorised: PathBuf,
}

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

/// `None` (with a message) when no server is configured; panics in CI, where a missing server
/// would otherwise let every test pass by skipping.
pub fn server() -> Option<TestServer> {
    let Ok(addr) = std::env::var("FERNJO_TEST_SSH_ADDR") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "FERNJO_TEST_SSH_ADDR is not set but CI is: the SSH test server must be running"
        );
        eprintln!("skipped: FERNJO_TEST_SSH_ADDR is not set (tools/ssh-test-server.sh start)");
        return None;
    };
    let (host, port) = addr.rsplit_once(':').expect("host:port");
    Some(TestServer {
        host: host.to_owned(),
        port: port.parse().expect("port"),
        addr,
        user: var("FERNJO_TEST_SSH_USER"),
        password: var("FERNJO_TEST_SSH_PASSWORD"),
        key_ed25519: var("FERNJO_TEST_SSH_KEY_ED25519").into(),
        key_rsa: var("FERNJO_TEST_SSH_KEY_RSA").into(),
        key_encrypted: var("FERNJO_TEST_SSH_KEY_ENCRYPTED").into(),
        key_passphrase: var("FERNJO_TEST_SSH_KEY_PASSPHRASE"),
        key_unauthorised: var("FERNJO_TEST_SSH_KEY_UNAUTHORISED").into(),
    })
}

#[derive(Default)]
struct State {
    host_keys: VecDeque<HostKeyDecision>,
    changed: VecDeque<HostKeyChangedDecision>,
    ki: VecDeque<Option<Vec<String>>>,
    passwords: VecDeque<Option<String>>,
    usernames: VecDeque<Option<String>>,
    passphrases: VecDeque<Option<String>>,
    host_key_prompts: Vec<HostKeyPrompt>,
    changed_prompts: Vec<HostKeyChangedPrompt>,
    ki_rounds: Vec<KiRound>,
    host_key_delay: std::time::Duration,
    host_key_calls: usize,
    changed_calls: usize,
    ki_calls: usize,
    password_calls: usize,
    username_calls: usize,
    passphrase_calls: usize,
}

/// Answers from queues; an empty queue answers "reject" / "cancel". Records every call.
#[derive(Default)]
pub struct ScriptedInteraction {
    state: Mutex<State>,
}

macro_rules! counter {
    ($name:ident) => {
        pub fn $name(&self) -> usize {
            self.state.lock().unwrap().$name
        }
    };
}

impl ScriptedInteraction {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn host_key(self, d: HostKeyDecision) -> Self {
        self.state.lock().unwrap().host_keys.push_back(d);
        self
    }

    /// Makes the host key prompt take this long to answer (a slow human).
    pub fn slow_host_key(self, d: std::time::Duration) -> Self {
        self.state.lock().unwrap().host_key_delay = d;
        self
    }

    pub fn host_key_changed(self, d: HostKeyChangedDecision) -> Self {
        self.state.lock().unwrap().changed.push_back(d);
        self
    }

    pub fn ki_answers(self, answers: Option<Vec<&str>>) -> Self {
        let a = answers.map(|v| v.into_iter().map(str::to_owned).collect());
        self.state.lock().unwrap().ki.push_back(a);
        self
    }

    pub fn typed_password(self, p: Option<&str>) -> Self {
        self.state
            .lock()
            .unwrap()
            .passwords
            .push_back(p.map(str::to_owned));
        self
    }

    pub fn username_answer(self, u: Option<&str>) -> Self {
        self.state
            .lock()
            .unwrap()
            .usernames
            .push_back(u.map(str::to_owned));
        self
    }

    pub fn passphrase(self, p: Option<&str>) -> Self {
        self.state
            .lock()
            .unwrap()
            .passphrases
            .push_back(p.map(str::to_owned));
        self
    }

    counter!(host_key_calls);
    counter!(changed_calls);
    counter!(ki_calls);
    counter!(password_calls);
    counter!(username_calls);
    counter!(passphrase_calls);

    pub fn host_key_prompts(&self) -> Vec<HostKeyPrompt> {
        self.state.lock().unwrap().host_key_prompts.clone()
    }

    pub fn changed_prompts(&self) -> Vec<HostKeyChangedPrompt> {
        self.state.lock().unwrap().changed_prompts.clone()
    }

    pub fn ki_rounds(&self) -> Vec<KiRound> {
        self.state.lock().unwrap().ki_rounds.clone()
    }
}

impl Interaction for ScriptedInteraction {
    async fn host_key(&self, prompt: HostKeyPrompt) -> HostKeyDecision {
        let delay = {
            let mut s = self.state.lock().unwrap();
            s.host_key_calls += 1;
            s.host_key_prompts.push(prompt);
            s.host_key_delay
        };
        tokio::time::sleep(delay).await;
        let mut s = self.state.lock().unwrap();
        s.host_keys.pop_front().unwrap_or(HostKeyDecision::Reject)
    }

    async fn host_key_changed(&self, prompt: HostKeyChangedPrompt) -> HostKeyChangedDecision {
        let mut s = self.state.lock().unwrap();
        s.changed_calls += 1;
        s.changed_prompts.push(prompt);
        s.changed
            .pop_front()
            .unwrap_or(HostKeyChangedDecision::Reject)
    }

    async fn keyboard_interactive(&self, round: KiRound) -> Option<Vec<SecretString>> {
        let mut s = self.state.lock().unwrap();
        s.ki_calls += 1;
        s.ki_rounds.push(round);
        s.ki.pop_front()
            .flatten()
            .map(|v| v.into_iter().map(SecretString::from).collect())
    }

    async fn password(&self, _user: &str, _host: &str) -> Option<SecretString> {
        let mut s = self.state.lock().unwrap();
        s.password_calls += 1;
        s.passwords.pop_front().flatten().map(SecretString::from)
    }

    async fn username(&self, _host: &str) -> Option<String> {
        let mut s = self.state.lock().unwrap();
        s.username_calls += 1;
        s.usernames.pop_front().flatten()
    }

    async fn key_passphrase(&self) -> Option<SecretString> {
        let mut s = self.state.lock().unwrap();
        s.passphrase_calls += 1;
        s.passphrases.pop_front().flatten().map(SecretString::from)
    }
}
