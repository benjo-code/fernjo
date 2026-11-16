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
