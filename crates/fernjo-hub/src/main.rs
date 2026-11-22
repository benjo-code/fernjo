//! `fernjo-hub init|serve|rotate-secret`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use fernjo_hub::{HubStore, config, router, serve, shutdown_signal};

const USAGE: &str = "usage:\n  fernjo-hub init --data <dir>\n  fernjo-hub serve --data <dir> --listen <addr:port>\n  fernjo-hub rotate-secret --data <dir>";

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("fernjo-hub: {msg}");
    ExitCode::FAILURE
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        return fail(USAGE);
    };
    let Some(data) = flag(&args, "--data").map(PathBuf::from) else {
        return fail(USAGE);
    };
    match cmd {
        "init" => match config::init(&data) {
            Ok(secret) => {
                println!("Hub secret (shown once, store it safely): {secret}");
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e.to_string()),
        },
        "rotate-secret" => match config::rotate_secret(&data) {
            Ok(secret) => {
                println!("New hub secret (shown once): {secret}");
                println!("Restart `fernjo-hub serve` so that it uses the new secret.");
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e.to_string()),
        },
        "serve" => {
            let Some(addr) = flag(&args, "--listen").and_then(|s| s.parse::<SocketAddr>().ok())
            else {
                return fail(USAGE);
            };
            let hash = match config::load_secret_hash(&data) {
                Ok(h) => h,
                Err(e) => return fail(&e.to_string()),
            };
            let store = match HubStore::open(&data) {
                Ok(s) => Arc::new(s),
                Err(e) => return fail(&e.to_string()),
            };
            match serve(addr, router(store, hash)).await {
                Ok(bound) => {
                    eprintln!("fernjo-hub listening on {bound}");
                    shutdown_signal().await;
                    // returning from `main` drops the runtime, which waits for a write that is
                    // already running on the blocking pool before the process exits
                    eprintln!("fernjo-hub stopping");
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e.to_string()),
            }
        }
        _ => fail(USAGE),
    }
}
