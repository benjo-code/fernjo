//! Automatic updates (M13). Pure decisions live in `decide`.

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the updater in M13 task 4/5")
)]
pub(crate) mod decide;
