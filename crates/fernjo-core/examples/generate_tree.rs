//! Writes a deterministic nested sample tree for manual checks (filter lag, restart).
//!
//! `cargo run -p fernjo-core --example generate_tree -- <dir> [count=500]`
//!
//! Start the app with `FERNJO_DATA_DIR=<dir>` to use the generated file.

use std::path::PathBuf;
use std::process::ExitCode;

use fernjo_core::{FILE_NAME, NodePatch, Patch, Protocol, Tree, save};

const TOP_FOLDERS: usize = 10;
const SUB_FOLDERS: usize = 5;
const WORDS: [&str; 8] = [
    "database",
    "web frontend",
    "backup",
    "monitoring",
    "build agent",
    "gateway",
    "lab",
    "archive",
];

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next().map(PathBuf::from) else {
        eprintln!("usage: generate_tree <dir> [count=500]");
        return ExitCode::from(2);
    };
    let count: usize = match args.next() {
        None => 500,
        Some(text) => match text.parse() {
            Ok(n) => n,
            Err(_) => {
                eprintln!("count must be a whole number, got {text:?}");
                return ExitCode::from(2);
            }
        },
    };

    let mut tree = Tree::new();
    let mut leaves = Vec::new();
    for top in 0..TOP_FOLDERS {
        let top_id = tree
            .create_folder(None, None, &format!("Site {:02}", top + 1))
            .expect("create top folder");
        if top % 3 == 0 {
            tree.update(
                top_id,
                NodePatch {
                    username: Patch::Set("ops".to_string()),
                    ..NodePatch::default()
                },
            )
            .expect("set folder username");
        }
        for sub in 0..SUB_FOLDERS {
            let sub_id = tree
                .create_folder(Some(top_id), None, &format!("Rack {}", sub + 1))
                .expect("create sub folder");
            if sub == 0 {
                tree.update(
                    sub_id,
                    NodePatch {
                        port: Patch::Set(2222),
                        ..NodePatch::default()
                    },
                )
                .expect("set folder port");
            }
            leaves.push(sub_id);
        }
    }

    for n in 0..count {
        let parent = leaves[n % leaves.len()];
        let protocol = if n % 4 == 3 {
            Protocol::Rdp
        } else {
            Protocol::Ssh
        };
        let name = format!("host-{:03}", n + 1);
        let id = tree
            .create_connection(
                Some(parent),
                None,
                &name,
                protocol,
                &format!("{name}.lab.example"),
            )
            .expect("create connection");
        tree.update(
            id,
            NodePatch {
                description: Some(WORDS[n % WORDS.len()].to_string()),
                ..NodePatch::default()
            },
        )
        .expect("set description");
    }

    let path = dir.join(FILE_NAME);
    if let Err(e) = save(&path, &tree) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    println!(
        "wrote {count} connections in {} folders to {}",
        TOP_FOLDERS * (SUB_FOLDERS + 1),
        path.display()
    );
    ExitCode::SUCCESS
}
