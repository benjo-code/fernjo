//! TLS trust of the client against a rustls stub with a throwaway CA: the pinned CA is accepted,
//! another CA and the system roots are refused as `hub_tls_untrusted` (not as unreachable).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

use fernjo_sync::client::{HubClient, HubConfig};
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use secrecy::SecretString;

struct Ca {
    pem: String,
    params: CertificateParams,
    key: KeyPair,
}

fn new_ca(name: &str) -> Ca {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, name);
    let key = KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    Ca {
        pem: cert.pem(),
        params,
        key,
    }
}

/// Serves `GET /v1/info` over TLS on 127.0.0.1 with a leaf signed by `ca`; returns the URL.
fn tls_stub(ca: &Ca) -> String {
    let leaf_key = KeyPair::generate().unwrap();
    let leaf_params = CertificateParams::new(vec!["127.0.0.1".to_owned()]).unwrap();
    let issuer = Issuer::new(ca.params.clone(), &ca.key);
    let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(leaf.der().to_vec())],
                PrivateKeyDer::from(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
            )
            .unwrap();
    let config = Arc::new(config);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let config = Arc::clone(&config);
            thread::spawn(move || {
                let Ok(conn) = ServerConnection::new(config) else {
                    return;
                };
                let mut tls = StreamOwned::new(conn, stream);
                let mut seen = Vec::new();
                let mut byte = [0u8; 1];
                while !seen.ends_with(b"\r\n\r\n") {
                    if tls.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    seen.push(byte[0]);
                }
                let body = b"{\"protocol\":1,\"maxBytes\":67108864}";
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = tls.write_all(head.as_bytes());
                let _ = tls.write_all(body);
                let _ = tls.flush();
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });
    format!("https://127.0.0.1:{port}")
}

fn client(url: &str, ca_pem: Option<&str>) -> HubClient {
    HubClient::new(&HubConfig {
        url: url.to_owned(),
        secret: SecretString::from("s".to_owned()),
        ca_pem: ca_pem.map(str::to_owned),
    })
    .unwrap()
}

#[test]
fn the_pinned_ca_is_accepted() {
    let ca = new_ca("fernjo test CA");
    let url = tls_stub(&ca);
    let info = client(&url, Some(&ca.pem)).info().unwrap();
    assert_eq!(info.protocol, 1);
}

#[test]
fn a_different_ca_is_hub_tls_untrusted() {
    let ca = new_ca("fernjo test CA");
    let other = new_ca("another CA");
    let url = tls_stub(&ca);
    let err = client(&url, Some(&other.pem)).info().unwrap_err();
    assert_eq!(err.code(), "hub_tls_untrusted");
}

#[test]
fn the_system_roots_refuse_a_private_ca() {
    let ca = new_ca("fernjo test CA");
    let url = tls_stub(&ca);
    let err = client(&url, None).info().unwrap_err();
    assert_eq!(err.code(), "hub_tls_untrusted");
}

#[test]
fn a_ca_that_is_not_pem_is_invalid_ca() {
    let err = HubClient::new(&HubConfig {
        url: "https://127.0.0.1:1".to_owned(),
        secret: SecretString::from("s".to_owned()),
        ca_pem: Some("not a certificate".to_owned()),
    })
    .unwrap_err();
    assert_eq!(err.code(), "invalid_ca");
}
