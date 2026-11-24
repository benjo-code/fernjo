//! Regression test for the v0.2.0 defect: an RDP connect must not need any root certificate. Up to
//! M11 the TLS upgrade built rustls's root-store verifier and refused to start when the platform
//! store held no usable root, so every host failed with `tls_failed` on such a machine.
//!
//! `rustls-native-certs` reads only `SSL_CERT_FILE` / `SSL_CERT_DIR` when either is set, so an
//! empty `SSL_CERT_FILE` simulates an empty store on every platform. This is its own test binary
//! (one process, one test) so the variable cannot leak into the other tests.

mod support;

use fernjo_rdp::{CertDecision, Session};
use support::{Behaviour, FakeServer, Fixture, Script};

#[test]
fn a_windows_like_certificate_reaches_the_certificate_decision_with_an_empty_root_store() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.pem");
    std::fs::write(&empty, b"").unwrap();
    // SAFETY: the only test of this binary, and the runtime below (the only other code that
    // could read the environment) is started afterwards.
    unsafe {
        std::env::set_var("SSL_CERT_FILE", &empty);
        std::env::remove_var("SSL_CERT_DIR");
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let server = FakeServer::start(Behaviour::TlsWithCert(Fixture::Rsa2048Sha256)).await;
        let script = Script::new(CertDecision::Reject);
        let result = Session::connect(server.params(), script.clone()).await;
        let code = match &result {
            Ok(_) => "ok",
            Err(e) => e.code(),
        };
        assert_eq!(code, "certificate_rejected");
        assert_eq!(
            script.log(),
            vec![format!(
                "certificate 127.0.0.1:{} {}",
                server.addr.port(),
                server.fingerprint
            )]
        );
    });
}
