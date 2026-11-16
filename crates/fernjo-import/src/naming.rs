//! Naming of the folder an import creates.

/// `base`, or `base (2)`, `base (3)`, ... when the name is already in `taken`.
pub(crate) fn unique_name(base: &str, taken: &[String]) -> String {
    let mut name = base.to_owned();
    let mut n = 2;
    while taken.contains(&name) {
        name = format!("{base} ({n})");
        n += 1;
    }
    name
}
