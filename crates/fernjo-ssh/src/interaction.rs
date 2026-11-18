//! Questions the SSH layer asks the user while connecting.

use std::future::Future;

use secrecy::SecretString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyPrompt {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
}

/// The key is in the synced `known_hosts` but this device has not confirmed it (another device
/// or a file added it). `previous` are the fingerprints this device confirmed earlier for the
/// same host, port and key type when they differ from the presented key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeySyncedPrompt {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub previous: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyDecision {
    TrustOnce,
    TrustAlways,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyChangedPrompt {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub old: Vec<String>,
    pub new: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyChangedDecision {
    Replace,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KiPrompt {
    pub text: String,
    pub echo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KiRound {
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<KiPrompt>,
}

/// Implemented by the caller (the Tauri shell in production, a script in tests). Used through
/// generics, not `dyn`; the futures are `Send` so implementers can be driven from `tokio::spawn`.
pub trait Interaction: Send + Sync + 'static {
    fn host_key(&self, prompt: HostKeyPrompt) -> impl Future<Output = HostKeyDecision> + Send;
    /// A host key that the synced `known_hosts` has but this device has not confirmed. The
    /// default asks like an unknown host (so existing implementers keep compiling); the shell
    /// overrides it with wording that says where the key came from.
    fn host_key_synced(
        &self,
        prompt: HostKeySyncedPrompt,
    ) -> impl Future<Output = HostKeyDecision> + Send {
        self.host_key(HostKeyPrompt {
            host: prompt.host,
            port: prompt.port,
            key_type: prompt.key_type,
            fingerprint: prompt.fingerprint,
        })
    }
    fn host_key_changed(
        &self,
        prompt: HostKeyChangedPrompt,
    ) -> impl Future<Output = HostKeyChangedDecision> + Send;
    /// One answer per prompt, or `None` to cancel.
    fn keyboard_interactive(
        &self,
        round: KiRound,
    ) -> impl Future<Output = Option<Vec<SecretString>>> + Send;
    fn password(&self, user: &str, host: &str)
    -> impl Future<Output = Option<SecretString>> + Send;
    fn username(&self, host: &str) -> impl Future<Output = Option<String>> + Send;
    fn key_passphrase(&self) -> impl Future<Output = Option<SecretString>> + Send;
}
