//! The update feed client: one check against the pinned endpoint and one verified download,
//! both through `tauri-plugin-updater` (Docs/DECISIONS.md, "M13: update signing and the updater
//! plugin"). The signature check is the plugin's: `Update::download` verifies the bytes against
//! the pinned public key and, with `requireSignedVersion`, against the version the signature was
//! made for, before it returns anything.
//!
//! What the plugin does not do and this module adds: the pre-release / skipped-version filter
//! (`decide::is_offered`; the plugin's comparator is a plain `>`), a size cap (the plugin buffers
//! the whole body without limit), a stall timeout and a check timeout. The endpoint and the
//! public key come from the `plugins.updater` configuration, never from code.

use std::time::Duration;

use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::{Error as PluginError, Update, UpdaterExt};
use tokio::sync::mpsc::unbounded_channel;
use url::Url;

use super::decide;

/// Upper bound for one feed check (connect, redirects and body).
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// A download that receives no chunk for this long is dropped.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(60);

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
        // a status other than success surfaces as `ReleaseNotFound`
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
    let builder = app.updater_builder();
    #[cfg(feature = "update-test-hooks")]
    let builder = match endpoint_override {
        Some(url) => builder
            .endpoints(vec![url.clone()])
            .map_err(|e| check_error(&e))?,
        None => builder,
    };
    #[cfg(not(feature = "update-test-hooks"))]
    let _ = endpoint_override;
    let updater = builder.build().map_err(|e| check_error(&e))?;
    let found = tokio::time::timeout(CHECK_TIMEOUT, updater.check())
        .await
        .map_err(|_| FeedError::Unreachable)?
        .map_err(|e| check_error(&e))?;
    let Some(update) = found else {
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

/// `decide::within_download_cap` with the cap as a parameter.
fn within_cap(cap: u64, declared: Option<u64>, received: u64) -> bool {
    declared.is_none_or(|d| d <= cap) && received <= cap
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
        within_cap(limits.cap, declared_seen, received)
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
    fn the_parameterised_cap_agrees_with_the_decision_module() {
        let cap = decide::MAX_UPDATE_BYTES;
        for (declared, received) in [
            (None, 0),
            (Some(cap), cap),
            (Some(cap + 1), 0),
            (None, cap + 1),
            (Some(1), cap + 1),
        ] {
            assert_eq!(
                within_cap(cap, declared, received),
                decide::within_download_cap(declared, received),
                "{declared:?} {received}"
            );
        }
    }

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
