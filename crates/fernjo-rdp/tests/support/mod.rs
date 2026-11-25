//! Test doubles for the connect tests: a scripted `Interaction` and a fake server that speaks just
//! enough RDP (the X.224 negotiation and a TLS handshake) to reach the certificate decision.
//! By default it selects CredSSP (`HYBRID`), as a server with NLA does; it never runs CredSSP, so
//! the steps after the prompts (NLA, the session itself) need a real host; see `tests/connect.rs`.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use fernjo_rdp::{
    CertChangedDecision, CertChangedPrompt, CertDecision, CertPrompt, ConnectParams, Interaction,
};
use ironrdp_pdu::nego::{
    ConnectionConfirm, ConnectionRequest, FailureCode, ResponseFlags, SecurityProtocol,
};
use ironrdp_pdu::x224::X224;
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::{self, ServerConfig};

/// An `Interaction` that answers from a script and remembers what it was asked.
pub struct Script {
    pub certificate: CertDecision,
    pub changed: CertChangedDecision,
    pub username: Option<String>,
    pub password: Option<String>,
    pub log: Mutex<Vec<String>>,
}

impl Script {
    pub fn new(certificate: CertDecision) -> Arc<Script> {
        Script::with_changed(certificate, CertChangedDecision::Reject)
    }

    pub fn with_changed(certificate: CertDecision, changed: CertChangedDecision) -> Arc<Script> {
        Arc::new(Script {
            certificate,
            changed,
            username: None,
            password: None,
            log: Mutex::new(Vec::new()),
        })
    }

    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn note(&self, what: impl Into<String>) {
        self.log.lock().unwrap().push(what.into());
    }
}

impl Interaction for Script {
    async fn certificate(&self, prompt: CertPrompt) -> CertDecision {
        self.note(format!(
            "certificate {}:{} {}",
            prompt.host, prompt.port, prompt.fingerprint
        ));
        self.certificate
    }

    async fn certificate_changed(&self, prompt: CertChangedPrompt) -> CertChangedDecision {
        self.note(format!(
            "certificate_changed {}:{} old={} new={}",
            prompt.host, prompt.port, prompt.old, prompt.new
        ));
        self.changed
    }

    async fn username(&self, host: &str) -> Option<String> {
        self.note(format!("username {host}"));
        self.username.clone()
    }

    async fn password(&self, user: &str, host: &str) -> Option<SecretString> {
        self.note(format!("password {user}@{host}"));
        self.password.clone().map(SecretString::from)
    }
}

#[derive(Clone, Copy)]
pub enum Behaviour {
    /// X.224 negotiation selecting CredSSP (`HYBRID`), then a TLS handshake with a self-signed
    /// certificate; the connection is closed once the client has the certificate and sent its
    /// next message (the start of CredSSP).
    TlsThenClose,
    /// A server without NLA: the X.224 confirm selects this protocol (`SSL` for TLS-only, empty
    /// for standard RDP security), then the server records everything the client sends until it
    /// hangs up (`FakeServer::after_confirm`). No TLS handshake is started.
    Selects(SecurityProtocol),
    /// As `Selects`, but the server answers the request with a negotiation failure.
    RefusesWith(FailureCode),
    /// As `TlsThenClose`, but the server is an impostor: it presents the real server's public
    /// certificate and signs the TLS handshake with a different private key, which is all an
    /// attacker on the path can do (the certificate is public, its key is not).
    ImpersonatedTls,
    /// Accepts and never answers.
    Silent,
    /// Answers with text that is not RDP and closes.
    Garbage,
    /// As `TlsThenClose`, but with a certificate and key from `tests/fixtures`. The key signs the
    /// handshake properly; the certificate is presented as is, without the consistency check
    /// rustls's `with_single_cert` would run (it parses the certificate with webpki too).
    TlsWithCert(Fixture),
    /// As `ImpersonatedTls` with another key type: the server presents the fixture's (RSA)
    /// certificate and signs the handshake with its own ECDSA P-256 key.
    ImpersonatedWithCert(Fixture),
}

/// A certificate and key from `tests/fixtures` (see the README there).
#[derive(Clone, Copy, Debug)]
pub enum Fixture {
    /// RSA 2048, SHA-256, like the certificate Windows creates for RDP.
    Rsa2048Sha256,
    /// RSA 2048, the certificate itself signed with SHA-1.
    Rsa2048Sha1,
    /// RSA 2048, SHA-256, with an unknown extension marked critical (webpki refuses to parse it).
    CriticalExtension,
}

impl Fixture {
    /// The certificate (DER) and its private key.
    pub fn load(self) -> (Vec<u8>, PrivateKeyDer<'static>) {
        let (cert, key): (&[u8], &[u8]) = match self {
            Fixture::Rsa2048Sha256 => (
                include_bytes!("../fixtures/rsa2048-sha256.crt"),
                include_bytes!("../fixtures/rsa2048-sha256.key"),
            ),
            Fixture::Rsa2048Sha1 => (
                include_bytes!("../fixtures/rsa2048-sha1.crt"),
                include_bytes!("../fixtures/rsa2048-sha1.key"),
            ),
            Fixture::CriticalExtension => (
                include_bytes!("../fixtures/critical-ext.crt"),
                include_bytes!("../fixtures/critical-ext.key"),
            ),
        };
        (
            CertificateDer::from_pem_slice(cert).unwrap().to_vec(),
            PrivateKeyDer::from_pem_slice(key).unwrap(),
        )
    }
}

pub struct FakeServer {
    pub addr: SocketAddr,
    /// `sha256:<hex>` of the certificate the server presents.
    pub fingerprint: String,
    pub connections: Arc<AtomicUsize>,
    /// The protocols of every X.224 connection request the server read.
    pub requested: Arc<Mutex<Vec<SecurityProtocol>>>,
    /// `Selects` and `RefusesWith`: the bytes the client sent after the confirm, set once the
    /// client hung up.
    after: Arc<Mutex<Option<Vec<u8>>>>,
}

impl FakeServer {
    pub async fn start(behaviour: Behaviour) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (der, key) = match behaviour {
            Behaviour::TlsWithCert(fixture) | Behaviour::ImpersonatedWithCert(fixture) => {
                fixture.load()
            }
            _ => {
                let cert =
                    rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
                (
                    cert.cert.der().to_vec(),
                    PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der())),
                )
            }
        };
        let fingerprint = format!("sha256:{}", hex(&Sha256::digest(&der)));
        let builder =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth();
        let config = if let Behaviour::TlsWithCert(_) = behaviour {
            let signing = rustls::crypto::ring::sign::any_supported_type(&key).unwrap();
            let certified =
                rustls::sign::CertifiedKey::new(vec![CertificateDer::from(der)], signing);
            builder.with_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(certified)))
        } else if matches!(
            behaviour,
            Behaviour::ImpersonatedTls | Behaviour::ImpersonatedWithCert(_)
        ) {
            // rcgen's default key type is ECDSA P-256
            let other = rcgen::KeyPair::generate().unwrap();
            let other_key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(other.serialize_der()));
            let signing = rustls::crypto::ring::sign::any_supported_type(&other_key).unwrap();
            let certified =
                rustls::sign::CertifiedKey::new(vec![CertificateDer::from(der.to_vec())], signing);
            builder.with_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(certified)))
        } else {
            builder
                .with_single_cert(vec![CertificateDer::from(der.to_vec())], key)
                .unwrap()
        };
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let connections = Arc::new(AtomicUsize::new(0));
        let counter = connections.clone();
        let requested = Arc::new(Mutex::new(Vec::new()));
        let requests = requested.clone();
        let after = Arc::new(Mutex::new(None));
        let after_confirm = after.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let acceptor = acceptor.clone();
                let requests = requests.clone();
                let after_confirm = after_confirm.clone();
                tokio::spawn(async move {
                    if matches!(behaviour, Behaviour::Silent) {
                        std::future::pending::<()>().await;
                    }
                    // the client's X.224 connection request: a TPKT header, then the rest
                    let mut header = [0u8; 4];
                    if stream.read_exact(&mut header).await.is_err() {
                        return;
                    }
                    let len = usize::from(u16::from_be_bytes([header[2], header[3]]));
                    let mut rest = vec![0u8; len.saturating_sub(4)];
                    if stream.read_exact(&mut rest).await.is_err() {
                        return;
                    }
                    let request = [header.as_slice(), rest.as_slice()].concat();
                    if let Ok(X224(request)) =
                        ironrdp_core::decode::<X224<ConnectionRequest>>(&request)
                    {
                        requests.lock().unwrap().push(request.protocol);
                    }
                    if matches!(behaviour, Behaviour::Garbage) {
                        // a TPKT header with a version that does not exist, then noise
                        let _ = stream
                            .write_all(&[0x01, 0x00, 0x00, 0x08, 0xde, 0xad, 0xbe, 0xef])
                            .await;
                        return;
                    }
                    let confirm = match behaviour {
                        Behaviour::Selects(protocol) => ConnectionConfirm::Response {
                            flags: ResponseFlags::empty(),
                            protocol,
                        },
                        Behaviour::RefusesWith(code) => ConnectionConfirm::Failure { code },
                        _ => ConnectionConfirm::Response {
                            flags: ResponseFlags::empty(),
                            protocol: SecurityProtocol::HYBRID,
                        },
                    };
                    let bytes = ironrdp_core::encode_vec(&X224(confirm)).unwrap();
                    if stream.write_all(&bytes).await.is_err() {
                        return;
                    }
                    if matches!(behaviour, Behaviour::Selects(_) | Behaviour::RefusesWith(_)) {
                        // whatever the client sends now (a TLS ClientHello, a Client Info PDU)
                        let mut seen = Vec::new();
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(10),
                            stream.read_to_end(&mut seen),
                        )
                        .await;
                        *after_confirm.lock().unwrap() = Some(seen);
                        return;
                    }
                    let Ok(mut tls) = acceptor.accept(stream).await else {
                        return;
                    };
                    // the client has its certificate; read what it sends next, then hang up
                    let mut sink = [0u8; 1024];
                    let _ = tls.read(&mut sink).await;
                });
            }
        });
        FakeServer {
            addr,
            fingerprint,
            connections,
            requested,
            after,
        }
    }

    /// `Selects` and `RefusesWith`: the bytes the client sent after the X.224 confirm, once it
    /// hung up (panics when that takes more than a few seconds).
    pub async fn after_confirm(&self) -> Vec<u8> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(seen) = self.after.lock().unwrap().clone() {
                return seen;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the client did not hang up after the confirm"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    pub fn params(&self) -> ConnectParams {
        let mut p = ConnectParams::new(
            "127.0.0.1",
            self.addr.port(),
            "admin",
            SecretString::from("secret".to_string()),
        );
        p.connect_timeout = std::time::Duration::from_secs(5);
        p
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
