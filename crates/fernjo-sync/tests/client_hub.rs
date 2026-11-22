//! The client against the real hub (`fernjo-hub`, in process, on loopback).

use std::net::SocketAddr;
use std::sync::Arc;
use std::thread;

use fernjo_hub::{HubStore, config, router, serve};
use fernjo_sync::client::{Expected, HubClient, HubConfig};
use fernjo_sync::{GroupId, SyncKey, seal};
use secrecy::SecretString;
use tempfile::TempDir;

const SECRET: &str = "test-secret";

struct Hub {
    _dir: TempDir,
    addr: SocketAddr,
}

/// Runs the hub on its own thread with its own runtime; the client under test blocks.
fn start() -> Hub {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(HubStore::open(dir.path()).unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let addr = serve(
                "127.0.0.1:0".parse().unwrap(),
                router(store, config::hash_secret(SECRET)),
            )
            .await
            .unwrap();
            tx.send(addr).unwrap();
            std::future::pending::<()>().await;
        });
    });
    Hub {
        _dir: dir,
        addr: rx.recv().unwrap(),
    }
}

fn client(hub: &Hub, secret: &str) -> HubClient {
    HubClient::new(&HubConfig {
        url: format!("http://{}", hub.addr),
        secret: SecretString::from(secret.to_owned()),
        ca_pem: None,
    })
    .unwrap()
}

fn envelope(seq: u64) -> Vec<u8> {
    seal(
        &SyncKey::from_bytes([3; 32]),
        GroupId::from_u128(42),
        seq,
        b"{\"hello\":true}",
    )
    .unwrap()
}

#[test]
fn put_get_round_trip_against_the_real_hub() {
    let hub = start();
    let c = client(&hub, SECRET);
    let info = c.info().unwrap();
    assert_eq!(info.protocol, 1);
    assert_eq!(info.max_bytes, 64 * 1024 * 1024);
    assert_eq!(c.get_head().unwrap(), None);
    let first = envelope(1);
    c.put_head(Expected::Empty, 1, &first).unwrap();
    assert_eq!(c.get_head().unwrap(), Some((1, first)));
    let second = envelope(2);
    c.put_head(Expected::Seq(1), 2, &second).unwrap();
    assert_eq!(c.get_head().unwrap(), Some((2, second)));
}

#[test]
fn a_stale_put_is_hub_conflict() {
    let hub = start();
    let c = client(&hub, SECRET);
    c.put_head(Expected::Empty, 1, &envelope(1)).unwrap();
    // another device already wrote when we still believe the hub is empty
    let err = c.put_head(Expected::Empty, 1, &envelope(1)).unwrap_err();
    assert_eq!(err.code(), "hub_conflict");
    c.put_head(Expected::Seq(1), 2, &envelope(2)).unwrap();
    let err = c.put_head(Expected::Seq(1), 3, &envelope(3)).unwrap_err();
    assert_eq!(err.code(), "hub_conflict");
    assert_eq!(c.get_head().unwrap().unwrap().0, 2);
}

#[test]
fn a_wrong_secret_is_hub_unauthorized() {
    let hub = start();
    let c = client(&hub, "not-the-secret");
    assert_eq!(c.info().unwrap_err().code(), "hub_unauthorized");
    assert_eq!(c.get_head().unwrap_err().code(), "hub_unauthorized");
    assert_eq!(
        c.put_head(Expected::Empty, 1, &envelope(1))
            .unwrap_err()
            .code(),
        "hub_unauthorized"
    );
}

#[test]
fn hub_and_sync_header_parsers_agree() {
    let valid = envelope(77);
    let hub = fernjo_hub::peek_header(&valid).unwrap();
    let sync = fernjo_sync::peek_header(&valid).unwrap();
    assert_eq!(hub, sync, "group id and seq are read identically");

    let mut wrong_magic = valid.clone();
    wrong_magic[0] ^= 0xFF;
    let mut version_zero = valid.clone();
    version_zero[8..10].copy_from_slice(&0u16.to_be_bytes());
    let mut version_two = valid.clone();
    version_two[8..10].copy_from_slice(&2u16.to_be_bytes());
    for (name, bad) in [
        ("empty", Vec::new()),
        ("three bytes", vec![1, 2, 3]),
        ("magic only", valid[..8].to_vec()),
        ("33 bytes", valid[..33].to_vec()),
        ("wrong magic", wrong_magic),
        ("version 0", version_zero),
        ("version 2", version_two),
    ] {
        assert!(
            fernjo_hub::peek_header(&bad).is_none(),
            "hub accepted {name}"
        );
        assert!(
            fernjo_sync::peek_header(&bad).is_err(),
            "sync accepted {name}"
        );
    }

    // The one deliberate difference: the hub only needs the 34 header bytes, while the client
    // wants a whole envelope (header, nonce, tag), so it refuses a header-only body the hub stores.
    let header_only = &valid[..40];
    assert!(fernjo_hub::peek_header(header_only).is_some());
    assert!(fernjo_sync::peek_header(header_only).is_err());
}
