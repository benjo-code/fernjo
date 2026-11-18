//! russh client handler: decides about the server's host key before anything else is sent.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use russh::client::Handler;
use russh::keys::PublicKey;

use crate::SshError;
use crate::interaction::{
    HostKeyChangedDecision, HostKeyChangedPrompt, HostKeyDecision, HostKeyPrompt,
    HostKeySyncedPrompt, Interaction,
};
use crate::known_hosts::{HostKeyStatus, KnownHosts, fingerprint};

/// Why the handler refused a key. `check_server_key` can only answer yes or no, so the real reason
/// is parked here for `Session::connect` to turn into the matching `SshError`.
pub(crate) type Rejection = Arc<Mutex<Option<SshError>>>;

pub(crate) struct ClientHandler<I: Interaction> {
    pub host: String,
    pub port: u16,
    pub known_hosts: KnownHosts,
    /// Keys this device confirmed itself. `None`: the layer is off and `known_hosts` alone decides.
    pub confirmed: Option<KnownHosts>,
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
            HostKeyStatus::Known => self.decide_synced(key).await,
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
                            Ok(()) => {
                                self.confirm(key, false);
                                true
                            }
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
                            Ok(()) => {
                                self.confirm(key, true);
                                true
                            }
                            Err(e) => self.reject(e),
                        }
                    }
                    HostKeyChangedDecision::Reject => self.reject(SshError::HostKeyChangedRejected),
                }
            }
        }
    }

    /// The key is in the synced `known_hosts`. That file can be written by another device or a
    /// sync tool, so it is only trusted silently when this device confirmed the same key.
    async fn decide_synced(&self, key: &PublicKey) -> bool {
        let Some(confirmed) = &self.confirmed else {
            return true;
        };
        // An unreadable confirmed file counts as "not confirmed": it asks, it never trusts.
        let previous = match confirmed.check(&self.host, self.port, key) {
            Ok(HostKeyStatus::Known) => return true,
            Ok(HostKeyStatus::Unknown) | Err(_) => Vec::new(),
            Ok(HostKeyStatus::Changed { stored }) => stored,
        };
        let replace = !previous.is_empty();
        let prompt = HostKeySyncedPrompt {
            host: self.host.clone(),
            port: self.port,
            key_type: key.algorithm().to_string(),
            fingerprint: fingerprint(key),
            previous,
        };
        let decision = {
            let _prompting = Prompting::begin(&self.prompting);
            self.interaction.host_key_synced(prompt).await
        };
        match decision {
            HostKeyDecision::TrustOnce => true,
            HostKeyDecision::TrustAlways => {
                self.confirm(key, replace);
                true
            }
            HostKeyDecision::Reject => self.reject(SshError::HostKeyRejected),
        }
    }

    /// Records the key as confirmed on this device. Best effort: when the file cannot be written
    /// the connect goes on and the next one asks again.
    fn confirm(&self, key: &PublicKey, replace: bool) {
        let Some(confirmed) = &self.confirmed else {
            return;
        };
        let _ = if replace {
            confirmed.replace(&self.host, self.port, key)
        } else {
            confirmed.add(&self.host, self.port, key)
        };
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

#[cfg(test)]
mod tests;
