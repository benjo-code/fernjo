//! Automatic updates (M13). Pure decisions live in `decide`, the feed client in `feed`.

// Items of `decide` that later tasks start to use (the install decision, the release page
// address and `within_download_cap`, which `feed` mirrors with a parameterised cap).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the updater in M13 task 5")
)]
pub(crate) mod decide;
pub mod feed;
