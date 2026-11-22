//! The human-readable text of a merge's findings, for the notice the UI shows.

use super::Finding;

/// One line for the notice. Names only; nothing in a finding is a secret.
#[must_use]
pub fn describe(finding: &Finding) -> String {
    match finding {
        Finding::ConflictCopy { name, .. } => {
            format!("\"{name}\" was changed on two devices; your version was kept as a copy.")
        }
        Finding::KeptEdit { name, .. } => {
            format!("\"{name}\" was deleted on another device but changed here, so it was kept.")
        }
        Finding::PlacementKept { name, .. } => {
            format!(
                "\"{name}\" was placed differently on two devices; the other device's place was kept."
            )
        }
        Finding::MoveUndone { name, .. } => {
            format!("Moving \"{name}\" would have broken the tree, so the move was undone.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fernjo_core::NodeId;

    #[test]
    fn every_finding_names_the_item() {
        let id = NodeId::nil();
        let all = [
            Finding::ConflictCopy {
                original: id,
                copy: id,
                name: "web".into(),
            },
            Finding::KeptEdit {
                node: id,
                name: "web".into(),
            },
            Finding::PlacementKept {
                node: id,
                name: "web".into(),
            },
            Finding::MoveUndone {
                node: id,
                name: "web".into(),
            },
        ];
        for finding in &all {
            assert!(describe(finding).contains("\"web\""));
        }
    }
}
