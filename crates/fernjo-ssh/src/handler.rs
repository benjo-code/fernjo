//! russh client handler: decides about the server's host key before anything else is sent.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use russh::client::Handler;
use russh::keys::PublicKey;

use crate::SshError;
use crate::interaction::{
    HostKeyChangedDecision, HostKeyChangedPrompt, HostKeyDecision, HostKeyPrompt, Interaction,
};
use crate::known_hosts::{HostKeyStatus, KnownHosts, fingerprint};

/// Why the handler refused a key. `check_server_key` can only answer yes or no, so the real reason
/// is parked here for `Session::connect` to turn into the matching `SshError`.
pub(crate) type Rejection = Arc<Mutex<Option<SshError>>>;

pub(crate) struct ClientHandler<I: Interaction> {
    pub host: String,
    pub port: u16,
    pub known_hosts: KnownHosts,
    pub interaction: Arc<I>,
    pub rejection: Rejection,
    /// Set while a user decision is pending, so the handshake deadline does not count that time.
    pub prompting: Arc<AtomicBool>,
}

/// Raises the flag for the lifetime of the guard.
struct Prompting<'a>(&'a AtomicBool);

impl<'a> Prompting<'a> {
    fn begin(flag: &'a AtomicBool) -> Self {
        flag.store(true, Ordering::SeqCst);
        Self(flag)
    }
}

impl Drop for Prompting<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl<I: Interaction> ClientHandler<I> {
    fn reject(&self, error: SshError) -> bool {
        if let Ok(mut slot) = self.rejection.lock() {
            *slot = Some(error);
        }
        false
    }

    async fn decide(&self, key: &PublicKey) -> bool {
        let status = match self.known_hosts.check(&self.host, self.port, key) {
            Ok(status) => status,
            Err(e) => return self.reject(e),
        };
        match status {
            HostKeyStatus::Known => true,
            HostKeyStatus::Unknown => {
                let prompt = HostKeyPrompt {
                    host: self.host.clone(),
                    port: self.port,
                    key_type: key.algorithm().to_string(),
                    fingerprint: fingerprint(key),
                };
                let decision = {
                    let _prompting = Prompting::begin(&self.prompting);
                    self.interaction.host_key(prompt).await
                };
                match decision {
                    HostKeyDecision::TrustOnce => true,
                    HostKeyDecision::TrustAlways => {
                        match self.known_hosts.add(&self.host, self.port, key) {
                            Ok(()) => true,
                            Err(e) => self.reject(e),
                        }
                    }
                    HostKeyDecision::Reject => self.reject(SshError::HostKeyRejected),
                }
            }
            HostKeyStatus::Changed { stored } => {
                let prompt = HostKeyChangedPrompt {
                    host: self.host.clone(),
                    port: self.port,
                    key_type: key.algorithm().to_string(),
                    old: stored,
                    new: fingerprint(key),
                };
                let decision = {
                    let _prompting = Prompting::begin(&self.prompting);
                    self.interaction.host_key_changed(prompt).await
                };
                match decision {
                    HostKeyChangedDecision::Replace => {
                        match self.known_hosts.replace(&self.host, self.port, key) {
                            Ok(()) => true,
                            Err(e) => self.reject(e),
                        }
                    }
                    HostKeyChangedDecision::Reject => self.reject(SshError::HostKeyChangedRejected),
                }
            }
        }
    }
}

impl<I: Interaction> Handler for ClientHandler<I> {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        match server_public_key {
            russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => {
                Ok(self.decide(key).await)
            }
            // Host certificates are never advertised (`host_key_certificates` stays empty).
            russh::keys::PublicKeyOrCertificate::Certificate(_) => {
                Ok(self.reject(SshError::HostKeyRejected))
            }
        }
    }
}
