//! The update feed client: one check against the pinned endpoint and one verified download
//! (Docs/DECISIONS.md, "M13: update signing and the updater plugin" and "M14: fixes from the
//! v0.4.0 security review"). The signature check is the plugin's: `Update::download` verifies the
//! bytes against the pinned public key and against the version the signature was made for, before
//! it returns anything.
//!
//! What this module does itself: it fetches `latest.json` under a size cap and hands it to the
//! plugin (`Updater::restore_update`, which always requires a signed version), filters
//! pre-releases and the skipped version (`decide::is_offered`; the plugin's comparator is a plain
//! `>`), caps the download (the plugin buffers the whole body without limit) and adds a stall and
//! a check timeout. The endpoint and the public key come from the `plugins.updater`
//! configuration, never from code.

use std::time::Duration;

use reqwest::{StatusCode, header::ACCEPT};
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::{Error as PluginError, Update, UpdaterExt};
use tokio::sync::mpsc::unbounded_channel;
use url::Url;

use super::decide;

/// Upper bound for one feed check (connect, redirects and body).
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// A download that receives no chunk for this long is dropped.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(60);
/// The largest `latest.json` that is parsed. The release's feed holds four signatures and at most
/// 16 KiB of notes, a few tens of KiB in all; parsing costs many times the size of the body.
pub const MAX_FEED_BYTES: usize = 256 * 1024;
/// The user agent of the pinned plugin, which the check keeps sending.
const USER_AGENT: &str = "tauri-plugin-updater/2.14.1";

/// An update the feed announced and that passed `decide::is_offered`.
pub struct FeedUpdate {
    pub version: String,
    /// Release notes as plain text (`decide::clean_notes`).
    pub notes: String,
    pub(crate) handle: Update,
}

impl std::fmt::Debug for FeedUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeedUpdate")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// Why a feed step failed. The texts and codes are fixed: no URL, header or body is ever in them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedError {
    /// The feed could not be fetched (network, timeout, an HTTP error status).
    Unreachable,
    /// The feed answered but its content is not a usable manifest.
    BadFeed,
    /// The update is not one this device may take (not newer, a pre-release or skipped).
    NotOffered,
    /// The download was refused or stopped, with a fixed reason (`too_large`, `stalled`, `failed`).
    DownloadRefused(&'static str),
    /// The signature does not match the pinned key, the file or the announced version.
    BadSignature,
}

impl FeedError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unreachable => "feed_unreachable",
            Self::BadFeed => "feed_invalid",
            Self::NotOffered => "not_offered",
            Self::DownloadRefused(_) => "download_refused",
            Self::BadSignature => "signature_invalid",
        }
    }
}

impl std::fmt::Display for FeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unreachable => "The update server could not be reached.",
            Self::BadFeed => "The update information is not valid.",
            Self::NotOffered => "This update is not offered.",
            Self::DownloadRefused("too_large") => "The update download is too large.",
            Self::DownloadRefused("stalled") => "The update download stopped receiving data.",
            Self::DownloadRefused(_) => "The update could not be downloaded.",
            Self::BadSignature => "The update's signature is not valid.",
        })
    }
}

impl std::error::Error for FeedError {}

fn is_signature_error(e: &PluginError) -> bool {
    matches!(
        e,
        PluginError::Minisign(_)
            | PluginError::Base64(_)
            | PluginError::SignatureUtf8(_)
            | PluginError::SignedVersionMismatch { .. }
            | PluginError::MissingSignedVersion
    )
}

fn check_error(e: &PluginError) -> FeedError {
    match e {
        PluginError::Reqwest(r) if r.is_decode() => FeedError::BadFeed,
        PluginError::Serialization(_)
        | PluginError::Semver(_)
        | PluginError::UrlParse(_)
        | PluginError::TargetNotFound(_)
        | PluginError::TargetsNotFound(_) => FeedError::BadFeed,
        // anything else the builder or the manifest can raise
        _ => FeedError::Unreachable,
    }
}

fn download_error(e: &PluginError) -> FeedError {
    if is_signature_error(e) {
        FeedError::BadSignature
    } else {
        FeedError::DownloadRefused("failed")
    }
}

/// The endpoint of the `plugins.updater` configuration (exactly one, without template variables;
/// `updater_config_is_pinned`).
fn configured_endpoint<R: Runtime>(app: &AppHandle<R>) -> Result<Url, FeedError> {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|c| c.get("endpoints")?.get(0)?.as_str())
        .and_then(|s| Url::parse(s).ok())
        .ok_or(FeedError::BadFeed)
}

/// Fetches the manifest and stops at [`MAX_FEED_BYTES`]. The plugin's own `check` reads a body
/// without limit into a `serde_json::Value`, which costs about 190 bytes of memory per byte and is
/// kept while the update is on offer (M14 review #83). The request is the plugin's: user agent,
/// `Accept`, redirects, system proxy and TLS stack. `None` is `204 No Content`.
async fn fetch_manifest(endpoint: Url) -> Result<Option<Vec<u8>>, FeedError> {
    // the TLS stack is built without a provider; this is the one the plugin installs
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|_| FeedError::Unreachable)?;
    let mut response = client
        .get(endpoint)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| FeedError::Unreachable)?;
    if !response.status().is_success() {
        return Err(FeedError::Unreachable);
    }
    if response.status() == StatusCode::NO_CONTENT {
        return Ok(None);
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_FEED_BYTES as u64)
    {
        return Err(FeedError::BadFeed);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| FeedError::Unreachable)? {
        if body.len() + chunk.len() > MAX_FEED_BYTES {
            return Err(FeedError::BadFeed);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
}

/// Checks the pinned endpoint once. `Ok(None)` means nothing is offered (no newer release, a
/// pre-release, or the skipped version): the plugin's answer and `decide::is_offered` must both
/// agree before an update is returned.
///
/// `endpoint_override` replaces the configured endpoint and exists only with the cargo feature
/// `update-test-hooks`, which the release build never enables; without it the argument is ignored.
///
/// The plugin's own request timeout is deliberately not set: it would also bound the download.
pub async fn check<R: Runtime>(
    app: &AppHandle<R>,
    current: &str,
    skipped: Option<&str>,
    endpoint_override: Option<&Url>,
) -> Result<Option<FeedUpdate>, FeedError> {
    // The plugin's default `on_before_exit` hides every window and drops the single-instance lock
    // before it starts the Windows setup; a start that fails (antivirus, application control) would
    // leave the app running unseen. After a successful start the process exits at once anyway.
    let builder = app.updater_builder().on_before_exit(|| {});
    let updater = builder.build().map_err(|e| check_error(&e))?;
    #[cfg(feature = "update-test-hooks")]
    let overridden = endpoint_override.cloned();
    #[cfg(not(feature = "update-test-hooks"))]
    let overridden = {
        let _ = endpoint_override;
        None::<Url>
    };
    let endpoint = match overridden {
        Some(url) => url,
        None => configured_endpoint(app)?,
    };
    let body = tokio::time::timeout(CHECK_TIMEOUT, fetch_manifest(endpoint))
        .await
        .map_err(|_| FeedError::Unreachable)??;
    let Some(body) = body else {
        return Ok(None);
    };
    // `restore_update` takes a `Value`, the one place the project builds one from foreign bytes;
    // the cap above bounds what that costs
    let manifest: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| FeedError::BadFeed)?;
    let Some(update) = updater
        .restore_update(manifest)
        .map_err(|e| check_error(&e))?
    else {
        return Ok(None);
    };
    if !decide::is_offered(current, &update.version, skipped) {
        return Ok(None);
    }
    Ok(Some(FeedUpdate {
        version: update.version.clone(),
        notes: decide::clean_notes(update.body.as_deref().unwrap_or_default()),
        handle: update,
    }))
}

/// Size and time bounds of a download. `production()` is what the app uses; the tests pass lower
/// values so the suite stays fast and never sends 256 MiB.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub cap: u64,
    pub stall: Duration,
}

impl Limits {
    pub const fn production() -> Self {
        Self {
            cap: decide::MAX_UPDATE_BYTES,
            stall: STALL_TIMEOUT,
        }
    }
}

/// Downloads the update with the production limits and returns the bytes the plugin verified.
pub async fn download(
    update: &FeedUpdate,
    on_progress: impl Fn(u64, Option<u64>) + Send + 'static,
) -> Result<Vec<u8>, FeedError> {
    download_with(update, Limits::production(), on_progress).await
}

/// `download` with explicit limits. The plugin buffers the whole body and cannot be told to stop,
/// so the download future runs under a loop that drops it as soon as the declared length or the
/// received bytes pass the cap, or no chunk arrives for `limits.stall`. `on_progress` gets the size
/// of each chunk and the declared length (`Content-Length`, when the server sent one).
pub async fn download_with(
    update: &FeedUpdate,
    limits: Limits,
    on_progress: impl Fn(u64, Option<u64>) + Send + 'static,
) -> Result<Vec<u8>, FeedError> {
    let (tx, mut rx) = unbounded_channel::<(u64, Option<u64>)>();
    let download = update.handle.download(
        move |chunk, declared| {
            let _ = tx.send((chunk as u64, declared));
        },
        || {},
    );
    tokio::pin!(download);

    let mut received = 0u64;
    let mut declared_seen = None;
    let mut account = |chunk: u64, declared: Option<u64>| {
        received += chunk;
        declared_seen = declared.or(declared_seen);
        on_progress(chunk, declared);
        decide::within_download_cap(limits.cap, declared_seen, received)
    };
    loop {
        tokio::select! {
            result = &mut download => {
                // chunks the select did not get to yet
                while let Ok((chunk, declared)) = rx.try_recv() {
                    if !account(chunk, declared) {
                        return Err(FeedError::DownloadRefused("too_large"));
                    }
                }
                let bytes = result.map_err(|e| download_error(&e))?;
                if bytes.len() as u64 > limits.cap {
                    return Err(FeedError::DownloadRefused("too_large"));
                }
                return Ok(bytes);
            }
            Some((chunk, declared)) = rx.recv() => {
                if !account(chunk, declared) {
                    return Err(FeedError::DownloadRefused("too_large"));
                }
            }
            () = tokio::time::sleep(limits.stall) => {
                return Err(FeedError::DownloadRefused("stalled"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_fixed() {
        assert_eq!(FeedError::Unreachable.code(), "feed_unreachable");
        assert_eq!(FeedError::BadFeed.code(), "feed_invalid");
        assert_eq!(FeedError::NotOffered.code(), "not_offered");
        assert_eq!(
            FeedError::DownloadRefused("stalled").code(),
            "download_refused"
        );
        assert_eq!(FeedError::BadSignature.code(), "signature_invalid");
    }
}
