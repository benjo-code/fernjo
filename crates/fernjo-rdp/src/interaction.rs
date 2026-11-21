//! Questions the RDP layer asks the user while connecting.

use std::future::Future;

use secrecy::SecretString;

/// The server presented a certificate this device has not seen for `host:port`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertPrompt {
    pub host: String,
    pub port: u16,
    /// `sha256:<lowercase hex>` of the DER certificate.
    pub fingerprint: String,
}

/// The server presented a certificate that differs from the pinned one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertChangedPrompt {
    pub host: String,
    pub port: u16,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertDecision {
    TrustOnce,
    TrustAlways,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertChangedDecision {
    Replace,
    Reject,
}

/// Implemented by the caller (the Tauri shell in production, a script in tests). Used through
/// generics, not `dyn`; the futures are `Send` so implementers can be driven from `tokio::spawn`.
pub trait Interaction: Send + Sync + 'static {
    fn certificate(&self, prompt: CertPrompt) -> impl Future<Output = CertDecision> + Send;
    fn certificate_changed(
        &self,
        prompt: CertChangedPrompt,
    ) -> impl Future<Output = CertChangedDecision> + Send;
    fn username(&self, host: &str) -> impl Future<Output = Option<String>> + Send;
    fn password(&self, user: &str, host: &str)
    -> impl Future<Output = Option<SecretString>> + Send;
}
