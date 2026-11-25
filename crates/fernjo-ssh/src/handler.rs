//! russh client handler: decides about the server's host key before anything else is sent, and
//! refuses every channel the server opens.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use russh::client::{self, Handler, Msg};
use russh::keys::PublicKey;
use russh::{Channel, ChannelOpenFailure};

use crate::SshError;
use crate::interaction::{
    HostKeyChangedDecision, HostKeyChangedPrompt, HostKeyDecision, HostKeyPrompt,
    HostKeySyncedPrompt, Interaction,
};
use crate::known_hosts::{HostKeyStatus, KnownHosts, blocking, fingerprint};

/// Why the handler refused a key. `check_server_key` can only answer yes or no, so the real reason
/// is parked here for `Session::connect` to turn into the matching `SshError`.
pub(crate) type Rejection = Arc<Mutex<Option<SshError>>>;

/// What the user decided to remember about the key. Nothing is written inside `check_server_key`:
/// the decision is parked here and `Session::connect` applies it once the handshake succeeded.
/// (russh 0.65 already checks the key-exchange signature before it calls `check_server_key`, so
/// this is defence in depth: a failed handshake, for whatever reason, leaves the files alone.)
pub(crate) type PendingTrust = Arc<Mutex<Option<TrustAction>>>;

/// A key to remember once the handshake is verified.
pub(crate) enum TrustAction {
    /// Unknown host, "trust always": add to `known_hosts`, then confirm on this device.
    Add(PublicKey),
    /// Changed key, "replace": replace in `known_hosts`, then confirm on this device.
    Replace(PublicKey),
    /// The key is already in `known_hosts`; only this device confirms it.
    Confirm(PublicKey),
    /// `known_hosts` lost the line of a key this device already confirmed: write it back, best
    /// effort. A failed write is ignored (read-only data folder, full disk), because this device's
    /// own record already vouches for exactly this key.
    Restore(PublicKey),
}

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
    /// The decision to write once the handshake is verified (see [`PendingTrust`]).
    pub pending_trust: PendingTrust,
}

impl<I: Interaction> Clone for ClientHandler<I> {
    fn clone(&self) -> Self {
        Self {
            host: self.host.clone(),
            port: self.port,
            known_hosts: self.known_hosts.clone(),
            confirmed: self.confirmed.clone(),
            interaction: Arc::clone(&self.interaction),
            rejection: Arc::clone(&self.rejection),
            prompting: Arc::clone(&self.prompting),
            pending_trust: Arc::clone(&self.pending_trust),
        }
    }
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

    /// `store.check` for this host and port, on the blocking pool (see `known_hosts::blocking`).
    async fn check_in(
        &self,
        store: &KnownHosts,
        key: &PublicKey,
    ) -> Result<HostKeyStatus, SshError> {
        let (store, host, port, key) = (store.clone(), self.host.clone(), self.port, key.clone());
        blocking(move || store.check(&host, port, &key)).await
    }

    /// Adds (`replace: false`) or replaces the key in `known_hosts`, on the blocking pool.
    async fn write_known(&self, key: PublicKey, replace: bool) -> Result<(), SshError> {
        let (store, host, port) = (self.known_hosts.clone(), self.host.clone(), self.port);
        blocking(move || {
            if replace {
                store.replace(&host, port, &key)
            } else {
                store.add(&host, port, &key)
            }
        })
        .await
    }

    /// The keys this device confirmed for the host and port, of every algorithm. Empty when the
    /// layer is off or nothing is confirmed; an unreadable confirmed file also counts as empty
    /// (it never makes a key trusted, see `decide_synced`).
    async fn confirmed_keys(&self) -> Vec<PublicKey> {
        let Some(confirmed) = &self.confirmed else {
            return Vec::new();
        };
        let (store, host, port) = (confirmed.clone(), self.host.clone(), self.port);
        blocking(move || store.stored_keys(&host, port))
            .await
            .unwrap_or_default()
    }

    async fn decide(&self, key: &PublicKey) -> bool {
        if let Ok(mut slot) = self.pending_trust.lock() {
            *slot = None;
        }
        let status = match self.check_in(&self.known_hosts, key).await {
            Ok(status) => status,
            Err(e) => return self.reject(e),
        };
        match status {
            HostKeyStatus::Known => self.decide_synced(key).await,
            HostKeyStatus::Unknown => {
                // `known_hosts` is synced and can lose the host's lines; this device's own record
                // decides whether this is a first contact (M11 review #24).
                let confirmed = self.confirmed_keys().await;
                if confirmed.contains(key) {
                    // this device confirmed exactly this key: `known_hosts` only lost its line
                    // (sync, a hand edit), so it is written back after the handshake, silently and
                    // best effort
                    return self.park(TrustAction::Restore(key.clone()));
                }
                if !confirmed.is_empty() {
                    let old = confirmed.iter().map(fingerprint).collect();
                    return self
                        .ask_changed(key, old, TrustAction::Replace(key.clone()))
                        .await;
                }
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
                    HostKeyDecision::TrustAlways => self.park(TrustAction::Add(key.clone())),
                    HostKeyDecision::Reject => self.reject(SshError::HostKeyRejected),
                }
            }
            HostKeyStatus::Changed { stored } => {
                self.ask_changed(key, stored, TrustAction::Replace(key.clone()))
                    .await
            }
        }
    }

    /// The changed-key warning: `old` are the fingerprints the presented key differs from.
    /// "Replace" parks `on_replace`.
    async fn ask_changed(
        &self,
        key: &PublicKey,
        old: Vec<String>,
        on_replace: TrustAction,
    ) -> bool {
        let prompt = HostKeyChangedPrompt {
            host: self.host.clone(),
            port: self.port,
            key_type: key.algorithm().to_string(),
            old,
            new: fingerprint(key),
        };
        let decision = {
            let _prompting = Prompting::begin(&self.prompting);
            self.interaction.host_key_changed(prompt).await
        };
        match decision {
            HostKeyChangedDecision::Replace => self.park(on_replace),
            HostKeyChangedDecision::Reject => self.reject(SshError::HostKeyChangedRejected),
        }
    }

    /// The key is in the synced `known_hosts`. That file can be written by another device or a
    /// sync tool, so it is only trusted silently when this device confirmed the same key. When
    /// this device confirmed other keys for the host (of any type), the key counts as changed:
    /// the warning names the confirmed fingerprints (M11 review #24). Only a host this device has
    /// confirmed nothing for gets the milder "came from another device" prompt.
    async fn decide_synced(&self, key: &PublicKey) -> bool {
        if self.confirmed.is_none() {
            return true;
        }
        // An unreadable confirmed file counts as "not confirmed": it asks, it never trusts.
        let confirmed = self.confirmed_keys().await;
        if confirmed.contains(key) {
            return true;
        }
        if !confirmed.is_empty() {
            let old = confirmed.iter().map(fingerprint).collect();
            // `known_hosts` already has the key: "replace" only confirms it on this device
            return self
                .ask_changed(key, old, TrustAction::Confirm(key.clone()))
                .await;
        }
        let prompt = HostKeySyncedPrompt {
            host: self.host.clone(),
            port: self.port,
            key_type: key.algorithm().to_string(),
            fingerprint: fingerprint(key),
        };
        let decision = {
            let _prompting = Prompting::begin(&self.prompting);
            self.interaction.host_key_synced(prompt).await
        };
        match decision {
            HostKeyDecision::TrustOnce => true,
            HostKeyDecision::TrustAlways => self.park(TrustAction::Confirm(key.clone())),
            HostKeyDecision::Reject => self.reject(SshError::HostKeyRejected),
        }
    }

    /// Remembers the decision for [`Self::apply_pending_trust`] and accepts the key for the
    /// handshake.
    fn park(&self, action: TrustAction) -> bool {
        if let Ok(mut slot) = self.pending_trust.lock() {
            *slot = Some(action);
        }
        true
    }

    /// Writes the parked decision. Called by `Session::connect` after the handshake succeeded;
    /// a failed handshake never calls it, so a peer that merely offered a key cannot get it
    /// recorded. A failed write of `known_hosts` after a user's decision (`Add`, `Replace`) is an
    /// error (the connect stops); the write-back of `Restore` and the device-confirmed file are
    /// best effort.
    pub(crate) async fn apply_pending_trust(&self) -> Result<(), SshError> {
        let action = self.pending_trust.lock().ok().and_then(|mut s| s.take());
        match action {
            None => Ok(()),
            Some(TrustAction::Add(key)) => {
                self.write_known(key.clone(), false).await?;
                self.confirm(&key).await;
                Ok(())
            }
            Some(TrustAction::Replace(key)) => {
                self.write_known(key.clone(), true).await?;
                self.confirm(&key).await;
                Ok(())
            }
            Some(TrustAction::Confirm(key)) => {
                self.confirm(&key).await;
                Ok(())
            }
            Some(TrustAction::Restore(key)) => {
                // the key is in the confirmed file already: nothing to confirm, and a failed
                // write-back only means the next connect restores it again
                let _ = self.write_known(key, false).await;
                Ok(())
            }
        }
    }

    /// Records the key as confirmed on this device. Best effort: when the file cannot be written
    /// the connect goes on and the next one asks again.
    async fn confirm(&self, key: &PublicKey) {
        let Some(confirmed) = &self.confirmed else {
            return;
        };
        // `trust` replaces a stored key of the same algorithm, so an earlier confirmation of a
        // different key (or an unreadable-then-recovered file) never leaves a stale one behind.
        let (store, host, port, key) =
            (confirmed.clone(), self.host.clone(), self.port, key.clone());
        let _ = blocking(move || store.trust(&host, port, &key)).await;
    }
}

/// Refuses a channel the server opened. fernjo asks for no forwarding (no X11, agent, TCP or
/// socket forwarding), so a server-opened channel is never expected; russh's default would accept
/// it and keep it, unread, until the server closes it (M11 review #23).
async fn refuse(reply: client::ChannelOpenHandle) -> Result<(), russh::Error> {
    reply
        .reject(ChannelOpenFailure::AdministrativelyProhibited)
        .await;
    Ok(())
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

    // Every channel type a server can open is refused. Unknown types are already refused by
    // russh (`should_accept_unknown_server_channel` defaults to false).

    async fn server_channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        refuse(reply).await
    }

    async fn server_channel_open_x11(
        &mut self,
        _channel: Channel<Msg>,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        refuse(reply).await
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        _channel: Channel<Msg>,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        refuse(reply).await
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        _channel: Channel<Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        refuse(reply).await
    }

    async fn server_channel_open_direct_tcpip(
        &mut self,
        _channel: Channel<Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        refuse(reply).await
    }

    async fn server_channel_open_forwarded_streamlocal(
        &mut self,
        _channel: Channel<Msg>,
        _socket_path: &str,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        refuse(reply).await
    }

    async fn server_channel_open_direct_streamlocal(
        &mut self,
        _channel: Channel<Msg>,
        _socket_path: &str,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        refuse(reply).await
    }
}

#[cfg(test)]
mod tests;
