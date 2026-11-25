//! Connect tests. The certificate decision and the failure paths run against an in-process fake
//! server (`tests/support`). Everything after the certificate (NLA, the session itself) needs a
//! real RDP host: an in-process `ironrdp-server` cannot be used because its crates pin another
//! `picky` release candidate than the client crates and the two do not resolve together. Those
//! tests read `FERNJO_TEST_RDP_ADDR` (`host:port`), `FERNJO_TEST_RDP_USER`,
//! `FERNJO_TEST_RDP_PASSWORD` and optionally `FERNJO_TEST_RDP_DOMAIN`, and skip with a message
//! when the address is unset, also in CI (there is no RDP host in CI; this is an owner check):
//! `FERNJO_TEST_RDP_ADDR=host:3389 FERNJO_TEST_RDP_USER=... FERNJO_TEST_RDP_PASSWORD=... cargo test -p fernjo-rdp --test connect`

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use fernjo_rdp::{
    CertChangedDecision, CertCheck, CertDecision, CertStore, CloseReason, ConnectParams, RdpError,
    Session, SessionEvent,
};
use ironrdp_pdu::nego::{FailureCode, SecurityProtocol};
use secrecy::SecretString;
use support::{Behaviour, FakeServer, Fixture, Script};

type Connected = Result<(Session, tokio::sync::mpsc::Receiver<SessionEvent>), RdpError>;

async fn connect(params: ConnectParams, script: &Arc<Script>) -> Connected {
    Session::connect(params, script.clone()).await
}

fn code(result: &Connected) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(e) => e.code(),
    }
}

#[tokio::test]
async fn unreachable_host_is_connect_failed_within_the_timeout() {
    // a port nobody listens on: bind, remember, close
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut params =
        ConnectParams::new("127.0.0.1", port, "u", SecretString::from("p".to_string()));
    params.connect_timeout = Duration::from_secs(5);
    let started = Instant::now();
    let result = connect(params, &Script::new(CertDecision::Reject)).await;
    assert_eq!(code(&result), "connect_failed");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_silent_server_times_out() {
    let server = FakeServer::start(Behaviour::Silent).await;
    let mut params = server.params();
    params.connect_timeout = Duration::from_millis(400);
    let started = Instant::now();
    let result = connect(params, &Script::new(CertDecision::Reject)).await;
    assert_eq!(code(&result), "timeout");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_server_that_is_not_rdp_is_a_protocol_error() {
    let server = FakeServer::start(Behaviour::Garbage).await;
    let result = connect(server.params(), &Script::new(CertDecision::Reject)).await;
    assert_eq!(code(&result), "protocol");
}

#[tokio::test]
async fn an_invalid_host_is_refused_before_any_connection() {
    for bad in ["", "a b", "a\nb", &"h".repeat(256)] {
        let params = ConnectParams::new(bad, 3389, "u", SecretString::from("p".to_string()));
        let result = connect(params, &Script::new(CertDecision::Reject)).await;
        assert_eq!(code(&result), "invalid_host", "{bad:?}");
    }
}

#[tokio::test]
async fn a_server_that_cannot_sign_for_its_certificate_fails_the_handshake_before_any_prompt() {
    // the pin is the hash of a public certificate; without a checked handshake signature an
    // attacker on the path could replay the real certificate and pass it
    let server = FakeServer::start(Behaviour::ImpersonatedTls).await;
    let params = server.params();
    let certs = params.certs.clone();
    let script = Script::new(CertDecision::TrustAlways);
    let result = connect(params, &script).await;
    assert_eq!(code(&result), "tls_failed");
    assert!(script.log().is_empty(), "{:?}", script.log());
    assert_eq!(
        certs
            .check("127.0.0.1", server.addr.port(), &server.fingerprint)
            .unwrap(),
        CertCheck::Unknown
    );
}

#[tokio::test]
async fn an_impostor_signing_with_another_key_type_fails_with_tls_failed_before_any_prompt() {
    // the replayed certificate is RSA, the impostor's key ECDSA: webpki calls the scheme
    // unsupported for the key, but the certificate is fine, so this must not read as
    // `certificate_unsupported`
    let server = FakeServer::start(Behaviour::ImpersonatedWithCert(Fixture::Rsa2048Sha256)).await;
    let params = server.params();
    let certs = params.certs.clone();
    let script = Script::new(CertDecision::TrustAlways);
    let result = connect(params, &script).await;
    assert_eq!(code(&result), "tls_failed");
    assert!(script.log().is_empty(), "{:?}", script.log());
    assert_eq!(
        certs
            .check("127.0.0.1", server.addr.port(), &server.fingerprint)
            .unwrap(),
        CertCheck::Unknown
    );
}

/// Connects to a server presenting `fixture`, rejects at the certificate decision and returns the
/// result, the prompts and the server.
async fn connect_with_fixture(fixture: Fixture) -> (&'static str, Vec<String>, FakeServer) {
    let server = FakeServer::start(Behaviour::TlsWithCert(fixture)).await;
    let script = Script::new(CertDecision::Reject);
    let result = connect(server.params(), &script).await;
    (code(&result), script.log(), server)
}

#[tokio::test]
async fn a_windows_like_rsa_certificate_reaches_the_certificate_decision() {
    // self-signed, no root store involved: the pin decides, as before
    let (code, log, server) = connect_with_fixture(Fixture::Rsa2048Sha256).await;
    assert_eq!(code, "certificate_rejected");
    assert_eq!(
        log,
        vec![format!(
            "certificate 127.0.0.1:{} {}",
            server.addr.port(),
            server.fingerprint
        )]
    );
}

#[tokio::test]
async fn a_sha1_signed_certificate_reaches_the_certificate_decision() {
    // the signature on the certificate is never checked (there is no chain to check it for); the
    // handshake signature is made with the certificate's RSA key and is checked as always
    let (code, log, server) = connect_with_fixture(Fixture::Rsa2048Sha1).await;
    assert_eq!(code, "certificate_rejected");
    assert_eq!(
        log,
        vec![format!(
            "certificate 127.0.0.1:{} {}",
            server.addr.port(),
            server.fingerprint
        )]
    );
}

#[tokio::test]
async fn a_certificate_webpki_cannot_parse_fails_with_certificate_unsupported_before_any_prompt() {
    let server = FakeServer::start(Behaviour::TlsWithCert(Fixture::CriticalExtension)).await;
    let params = server.params();
    let certs = params.certs.clone();
    let script = Script::new(CertDecision::TrustAlways);
    let result = connect(params, &script).await;
    assert_eq!(code(&result), "certificate_unsupported");
    assert!(script.log().is_empty(), "{:?}", script.log());
    assert_eq!(
        certs
            .check("127.0.0.1", server.addr.port(), &server.fingerprint)
            .unwrap(),
        CertCheck::Unknown
    );
}

#[tokio::test]
async fn a_new_certificate_asks_and_reject_ends_with_certificate_rejected() {
    let server = FakeServer::start(Behaviour::TlsThenClose).await;
    let params = server.params();
    let certs = params.certs.clone();
    let script = Script::new(CertDecision::Reject);
    let result = connect(params, &script).await;
    assert_eq!(code(&result), "certificate_rejected");
    assert_eq!(
        script.log(),
        vec![format!(
            "certificate 127.0.0.1:{} {}",
            server.addr.port(),
            server.fingerprint
        )]
    );
    // nothing was pinned, so the next attempt asks again
    assert_eq!(
        certs
            .check("127.0.0.1", server.addr.port(), &server.fingerprint)
            .unwrap(),
        CertCheck::Unknown
    );
}

#[tokio::test]
async fn trust_always_pins_and_a_pinned_certificate_connects_without_asking() {
    let server = FakeServer::start(Behaviour::TlsThenClose).await;
    let params = server.params();
    let certs = params.certs.clone();
    let first = Script::new(CertDecision::TrustAlways);
    let result = connect(params, &first).await;
    // the fake server ends the connection after the TLS handshake, so the connect fails later
    assert_ne!(code(&result), "certificate_rejected");
    assert_eq!(first.log().len(), 1);
    let port = server.addr.port();
    assert_eq!(
        certs.check("127.0.0.1", port, &server.fingerprint).unwrap(),
        CertCheck::Match
    );

    let mut again = server.params();
    again.certs = certs;
    let second = Script::new(CertDecision::Reject);
    let result = connect(again, &second).await;
    assert_ne!(code(&result), "certificate_rejected");
    assert!(second.log().is_empty(), "{:?}", second.log());
}

#[tokio::test]
async fn trust_once_connects_but_does_not_pin() {
    let server = FakeServer::start(Behaviour::TlsThenClose).await;
    let params = server.params();
    let certs = params.certs.clone();
    let script = Script::new(CertDecision::TrustOnce);
    let result = connect(params, &script).await;
    assert_ne!(code(&result), "certificate_rejected");
    assert_eq!(
        certs
            .check("127.0.0.1", server.addr.port(), &server.fingerprint)
            .unwrap(),
        CertCheck::Unknown
    );
}

#[tokio::test]
async fn a_changed_certificate_asks_and_replace_pins_the_new_one() {
    let server = FakeServer::start(Behaviour::TlsThenClose).await;
    let port = server.addr.port();
    let certs = CertStore::in_memory();
    certs.pin("127.0.0.1", port, "sha256:0000").unwrap();

    // reject: the old pin stays
    let mut params = server.params();
    params.certs = certs.clone();
    let script = Script::new(CertDecision::Reject);
    let result = connect(params, &script).await;
    assert_eq!(code(&result), "certificate_rejected");
    assert_eq!(
        script.log(),
        vec![format!(
            "certificate_changed 127.0.0.1:{port} old=sha256:0000 new={}",
            server.fingerprint
        )]
    );
    assert_eq!(
        certs.check("127.0.0.1", port, "sha256:0000").unwrap(),
        CertCheck::Match
    );

    // replace: the new certificate is pinned and the connect goes on
    let mut params = server.params();
    params.certs = certs.clone();
    let script = Script::with_changed(CertDecision::Reject, CertChangedDecision::Replace);
    let result = connect(params, &script).await;
    assert_ne!(code(&result), "certificate_rejected");
    assert_eq!(
        certs.check("127.0.0.1", port, &server.fingerprint).unwrap(),
        CertCheck::Match
    );
}

#[tokio::test]
async fn the_password_is_asked_only_after_the_certificate_is_trusted() {
    let server = FakeServer::start(Behaviour::TlsThenClose).await;
    // rejected certificate: no password prompt at all
    let mut params = server.params();
    params.password = SecretString::from(String::new());
    let script = Script::new(CertDecision::Reject);
    let result = connect(params, &script).await;
    assert_eq!(code(&result), "certificate_rejected");
    assert!(
        script.log().iter().all(|l| !l.starts_with("password")),
        "{:?}",
        script.log()
    );

    // trusted certificate, cancelled password prompt: the attempt ends as a failed login
    let mut params = server.params();
    params.password = SecretString::from(String::new());
    let script = Script::new(CertDecision::TrustOnce);
    let result = connect(params, &script).await;
    assert_eq!(code(&result), "auth_failed");
    let log = script.log();
    assert!(log[0].starts_with("certificate "), "{log:?}");
    assert_eq!(log[1], "password admin@127.0.0.1");
}

#[tokio::test]
async fn an_empty_user_name_asks_for_one() {
    let server = FakeServer::start(Behaviour::TlsThenClose).await;
    let mut params = server.params();
    params.username = String::new();
    let script = Script::new(CertDecision::TrustOnce);
    let result = connect(params, &script).await;
    assert_eq!(code(&result), "auth_failed");
    assert_eq!(script.log().last().unwrap(), "username 127.0.0.1");
}

// ---- servers without NLA -----------------------------------------------------------------

/// Connects with an empty user name and password (so any prompt would be raised) to a server with
/// `behaviour`, and checks that the attempt ends with `nla_required` before anything follows the
/// X.224 confirm: no prompt, no pin, and not a byte from the client after the confirm (no TLS
/// ClientHello, no Client Info PDU).
async fn assert_refused_without_nla(behaviour: Behaviour) {
    let server = FakeServer::start(behaviour).await;
    let mut params = server.params();
    params.username = String::new();
    params.password = SecretString::from(String::new());
    let certs = params.certs.clone();
    let script = Script::new(CertDecision::TrustAlways);
    let result = connect(params, &script).await;
    assert_eq!(code(&result), "nla_required");
    assert!(script.log().is_empty(), "{:?}", script.log());
    assert!(
        server.after_confirm().await.is_empty(),
        "the client sent something after the confirm"
    );
    // no second attempt (with TLS, say) on another connection
    assert_eq!(server.connections.load(Ordering::SeqCst), 1);
    assert_eq!(
        certs
            .check("127.0.0.1", server.addr.port(), &server.fingerprint)
            .unwrap(),
        CertCheck::Unknown
    );
}

#[tokio::test]
async fn the_request_offers_credssp_and_not_tls_alone() {
    let server = FakeServer::start(Behaviour::TlsThenClose).await;
    let _ = connect(server.params(), &Script::new(CertDecision::Reject)).await;
    let requested = server.requested.lock().unwrap().clone();
    assert_eq!(
        requested,
        vec![SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX]
    );
}

#[tokio::test]
async fn a_tls_only_server_is_refused_with_nla_required_before_tls_and_any_prompt() {
    assert_refused_without_nla(Behaviour::Selects(SecurityProtocol::SSL)).await;
}

#[tokio::test]
async fn a_server_selecting_standard_rdp_security_is_refused_with_nla_required() {
    assert_refused_without_nla(Behaviour::Selects(SecurityProtocol::empty())).await;
}

#[tokio::test]
async fn any_other_selection_without_credssp_is_refused_with_nla_required() {
    // RDSTLS sends the password inside TLS like TLS-only; an unknown bit next to SSL is no CredSSP
    // either. fernjo refuses them itself instead of relying on IronRDP's generic check.
    assert_refused_without_nla(Behaviour::Selects(SecurityProtocol::RDSTLS)).await;
    assert_refused_without_nla(Behaviour::Selects(
        SecurityProtocol::SSL | SecurityProtocol::from_bits_retain(0x0000_0100),
    ))
    .await;
}

#[tokio::test]
async fn a_negotiation_failure_for_lack_of_nla_is_nla_required() {
    for failure in [
        FailureCode::SSL_REQUIRED_BY_SERVER,
        FailureCode::SSL_NOT_ALLOWED_BY_SERVER,
        FailureCode::SSL_WITH_USER_AUTH_REQUIRED_BY_SERVER,
    ] {
        assert_refused_without_nla(Behaviour::RefusesWith(failure)).await;
    }
}

#[tokio::test]
async fn another_negotiation_failure_stays_connect_failed() {
    // the server has no certificate: NLA would not help, so this is not `nla_required`
    let server =
        FakeServer::start(Behaviour::RefusesWith(FailureCode::SSL_CERT_NOT_ON_SERVER)).await;
    let script = Script::new(CertDecision::TrustAlways);
    let result = connect(server.params(), &script).await;
    assert_eq!(code(&result), "connect_failed");
    assert!(script.log().is_empty(), "{:?}", script.log());
    assert!(server.after_confirm().await.is_empty());
}

// ---- tests that need a real RDP host -----------------------------------------------------

struct Host {
    host: String,
    port: u16,
    user: String,
    password: String,
    domain: Option<String>,
}

/// The real host, or `None` (and a message) when `FERNJO_TEST_RDP_ADDR` is unset.
fn real_host(test: &str) -> Option<Host> {
    let Ok(addr) = std::env::var("FERNJO_TEST_RDP_ADDR") else {
        eprintln!("skipping {test}: FERNJO_TEST_RDP_ADDR is not set (needs a real RDP host)");
        return None;
    };
    let (host, port) = addr.rsplit_once(':').unwrap_or((addr.as_str(), "3389"));
    Some(Host {
        host: host.to_string(),
        port: port.parse().expect("a port"),
        user: std::env::var("FERNJO_TEST_RDP_USER").expect("FERNJO_TEST_RDP_USER"),
        password: std::env::var("FERNJO_TEST_RDP_PASSWORD").expect("FERNJO_TEST_RDP_PASSWORD"),
        domain: std::env::var("FERNJO_TEST_RDP_DOMAIN").ok(),
    })
}

fn real_params(h: &Host) -> ConnectParams {
    let mut p = ConnectParams::new(
        h.host.clone(),
        h.port,
        h.user.clone(),
        SecretString::from(h.password.clone()),
    );
    p.domain = h.domain.clone();
    p
}

#[tokio::test]
async fn connects_with_nla_and_receives_damage() {
    let Some(h) = real_host("connects_with_nla_and_receives_damage") else {
        return;
    };
    let script = Script::new(CertDecision::TrustOnce);
    let (session, mut events) = connect(real_params(&h), &script).await.expect("connect");
    let (w, hgt) = session.framebuffer().size();
    assert!(w >= 200 && hgt >= 200);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut damage = None;
    while Instant::now() < deadline {
        if let Ok(Some(SessionEvent::Damage(r))) =
            tokio::time::timeout(Duration::from_secs(1), events.recv()).await
        {
            damage = Some(r);
            break;
        }
    }
    let rect = damage.expect("the server painted something");
    let pixels = session.framebuffer().copy_region(rect);
    assert!(
        pixels.iter().any(|b| *b != 0),
        "the painted region is not blank"
    );
    session.close();
}

#[tokio::test]
async fn wrong_password_is_auth_failed() {
    let Some(h) = real_host("wrong_password_is_auth_failed") else {
        return;
    };
    let mut p = real_params(&h);
    p.password = SecretString::from("definitely-not-the-password".to_string());
    let script = Script::new(CertDecision::TrustOnce);
    let result = connect(p, &script).await;
    assert_eq!(code(&result), "auth_failed");
}

#[tokio::test]
async fn close_ends_with_closed_by_user() {
    let Some(h) = real_host("close_ends_with_closed_by_user") else {
        return;
    };
    let script = Script::new(CertDecision::TrustOnce);
    let (session, mut events) = connect(real_params(&h), &script).await.expect("connect");
    session.close();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
            Ok(Some(SessionEvent::Closed(reason))) => {
                assert_eq!(reason, CloseReason::ClosedByUser);
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("the event queue ended without a Closed event"),
            Err(_) => assert!(Instant::now() < deadline, "no Closed event after close"),
        }
    }
}

#[tokio::test]
async fn clipboard_text_round_trips_with_the_setting_on_and_not_with_it_off() {
    let Some(h) = real_host("clipboard_text_round_trips_with_the_setting_on_and_not_with_it_off")
    else {
        return;
    };
    // with the setting off, setting text is a no-op and never an error
    let mut off = real_params(&h);
    off.clipboard = false;
    let (session, _events) = connect(off, &Script::new(CertDecision::TrustOnce))
        .await
        .expect("connect");
    session.set_clipboard_text("ignored".into()).await.unwrap();
    session.close();
    // with it on, text can be offered; reading it back needs a person on the remote side, so the
    // round trip itself is part of the owner's manual check
    let (session, _events) = connect(real_params(&h), &Script::new(CertDecision::TrustOnce))
        .await
        .expect("connect");
    session
        .set_clipboard_text("hello from fernjo".into())
        .await
        .unwrap();
    session.close();
}
