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
            // The finding does not record which side deleted, so the text is true on both.
            format!("\"{name}\" was deleted on one device but changed on another, so it was kept.")
        }
        Finding::PlacementKept { name, .. } => {
            format!(
                "\"{name}\" was placed differently on two devices; the other device's place was kept."
            )
        }
        Finding::MoveUndone { name, .. } => {
            format!("Moving \"{name}\" would have broken the tree, so the move was undone.")
        }
        Finding::EntryConflictCopy { name, .. } => {
            format!(
                "Vault entry \"{name}\" was changed on two devices; your version was kept as a copy."
            )
        }
        Finding::EntryKeptEdit { name, .. } => {
            format!(
                "Vault entry \"{name}\" was deleted on one device but changed on another, so it was kept."
            )
        }
        Finding::PasswordChangeDropped { .. } => {
            "The master password you set on this device was replaced by the one from your other device. Change it again if you still want a different one."
                .to_owned()
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
            Finding::EntryConflictCopy {
                original: uuid::Uuid::nil(),
                copy: uuid::Uuid::nil(),
                name: "web".into(),
            },
            Finding::EntryKeptEdit {
                entry: uuid::Uuid::nil(),
                name: "web".into(),
            },
        ];
        for finding in &all {
            assert!(describe(finding).contains("\"web\""));
        }
    }

    #[test]
    fn the_kept_notices_read_true_on_the_deleting_and_the_editing_device() {
        let id = NodeId::nil();
        let node = describe(&Finding::KeptEdit {
            node: id,
            name: "web".into(),
        });
        let entry = describe(&Finding::EntryKeptEdit {
            entry: uuid::Uuid::nil(),
            name: "web".into(),
        });
        for text in [node, entry] {
            assert!(text.contains("deleted on one device but changed on another"));
            assert!(
                !text.contains("here"),
                "names a side it cannot know: {text}"
            );
        }
    }

    #[test]
    fn the_password_notice_says_what_to_do_and_names_no_secret() {
        let finding = Finding::PasswordChangeDropped {
            vault: uuid::Uuid::nil(),
        };
        let text = describe(&finding);
        assert!(text.contains("Change it again"));
        assert!(text.contains("master password"));
        assert!(format!("{finding:?}").starts_with("PasswordChangeDropped"));
    }
}
