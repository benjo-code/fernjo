//! The TLS upgrade of an RDP connection, with fernjo's own certificate verifier.
//!
//! RDP servers present self-signed certificates, so the verifier accepts any chain, name and
//! validity period: trust is decided after the handshake by the pin (`CertStore`, in
//! `connect.rs`). What it never skips is the handshake signature: the TLS 1.2 `ServerKeyExchange`
//! and the TLS 1.3 `CertificateVerify` must be signed with the private key of the presented
//! certificate, checked through rustls's own `verify_tls12_signature` / `verify_tls13_signature`
//! with the ring provider's algorithms. Without that, a server that replays a public certificate
//! would pass the pin (`Docs/DECISIONS.md`, "M8 RDP TLS handshake signature check" and "M11 RDP TLS:
//! own verifier, strict handshake signature").
//!
//! Those functions parse the certificate with webpki. A certificate webpki cannot parse, or whose
//! key the provider cannot verify with (for example RSA under 2048 bits), fails the handshake as
//! well; that failure is reported as `certificate_unsupported` instead of `tls_failed`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{
    CryptoProvider, WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature,
};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use x509_cert::der::asn1::{AnyRef, UintRef};
use x509_cert::der::{Decode as _, Reader as _, SliceReader};

use crate::RdpError;

/// RSA modulus sizes the ring provider verifies with (`RSA_PKCS1_2048_8192_*`,
/// `RSA_PSS_2048_8192_*` in rustls-webpki's `ring_algs`).
const RSA_BITS: std::ops::RangeInclusive<usize> = 2048..=8192;

/// `rsaEncryption`, 1.2.840.113549.1.1.1.
const RSA_ENCRYPTION: x509_cert::der::asn1::ObjectIdentifier =
    x509_cert::der::asn1::ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");

/// Accepts every certificate chain and checks every handshake signature strictly.
#[derive(Debug)]
pub(crate) struct PinVerifier {
    provider: Arc<CryptoProvider>,
    /// Set when a signature check failed because the certificate is one rustls cannot use.
    unsupported: Arc<AtomicBool>,
}

impl PinVerifier {
    fn note(
        &self,
        cert: &CertificateDer<'_>,
        result: Result<HandshakeSignatureValid, rustls::Error>,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        if let Err(error) = &result
            && is_unsupported(
                error,
                cert,
                &self.provider.signature_verification_algorithms,
            )
        {
            self.unsupported.store(true, Ordering::SeqCst);
        }
        result
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // no chain, name or expiry check: the pin decides after the handshake
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let result = verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        );
        self.note(cert, result)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let result = verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        );
        self.note(cert, result)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Whether a failed signature check is down to the certificate rather than the signature: webpki
/// cannot parse it (`rustls::server::ParsedCertificate`, the same parse the verify functions run),
/// the provider has no algorithm at all for its key type, or it is an RSA key outside the sizes
/// ring verifies (ring reports that as a bad signature).
///
/// A scheme that does not fit a key type the provider does support (an RSA certificate with an
/// ECDSA signature) is what an impostor produces when it replays a public certificate and signs
/// with its own key, so it stays a plain TLS failure.
fn is_unsupported(
    error: &rustls::Error,
    cert: &CertificateDer<'_>,
    algorithms: &WebPkiSupportedAlgorithms,
) -> bool {
    let Ok(parsed) = ParsedCertificate::try_from(cert) else {
        return true;
    };
    match error {
        rustls::Error::InvalidCertificate(
            CertificateError::UnsupportedSignatureAlgorithmForPublicKeyContext { .. },
        ) => !key_type_supported(&parsed, algorithms),
        rustls::Error::InvalidCertificate(CertificateError::BadSignature) => {
            rsa_key_out_of_range(cert)
        }
        _ => false,
    }
}

/// Whether the certificate's public key algorithm (the `AlgorithmIdentifier` of its SPKI, curve
/// included) is one some algorithm of the provider verifies with; the same comparison webpki's
/// `signed_data::verify_signature` makes against `public_key_alg_id`. A key that cannot be read
/// counts as supported, so the error stays `tls_failed`.
fn key_type_supported(
    cert: &ParsedCertificate<'_>,
    algorithms: &WebPkiSupportedAlgorithms,
) -> bool {
    let spki = cert.subject_public_key_info();
    let Some(algorithm) = spki_algorithm(spki.as_ref()) else {
        return true;
    };
    algorithms
        .all
        .iter()
        .any(|alg| alg.public_key_alg_id().as_ref() == algorithm)
}

/// The contents of the `AlgorithmIdentifier` of a DER `SubjectPublicKeyInfo`.
fn spki_algorithm(spki: &[u8]) -> Option<&[u8]> {
    let mut reader = SliceReader::new(spki).ok()?;
    reader
        .sequence(|seq| {
            let algorithm = AnyRef::decode(seq)?;
            seq.drain(seq.remaining_len())?;
            Ok::<_, x509_cert::der::Error>(algorithm.value())
        })
        .ok()
}

/// True for an RSA key whose modulus size or public exponent ring refuses to verify with
/// (modulus 2048 to 8192 bits, odd exponent 3 to 2^33 - 1). Only used to name the error.
fn rsa_key_out_of_range(cert: &CertificateDer<'_>) -> bool {
    let Ok(cert) = x509_cert::Certificate::from_der(cert.as_ref()) else {
        return false;
    };
    let spki = cert.tbs_certificate().subject_public_key_info();
    if spki.algorithm.oid != RSA_ENCRYPTION {
        return false;
    }
    let Some(key) = spki.subject_public_key.as_bytes() else {
        return false;
    };
    let Ok((modulus, exponent)) = rsa_public_key(key) else {
        return false;
    };
    let exponent_ok = exponent.len() <= 5 && {
        let value = exponent
            .iter()
            .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
        value % 2 == 1 && (3..=(1u64 << 33) - 1).contains(&value)
    };
    !RSA_BITS.contains(&bit_length(modulus)) || !exponent_ok
}

/// `RSAPublicKey ::= SEQUENCE { modulus INTEGER, publicExponent INTEGER }`: both as big-endian
/// bytes without leading zeros.
fn rsa_public_key(der: &[u8]) -> Result<(&[u8], &[u8]), x509_cert::der::Error> {
    let mut reader = SliceReader::new(der)?;
    let pair = reader.sequence(|seq| {
        let modulus = UintRef::decode(seq)?;
        let exponent = UintRef::decode(seq)?;
        Ok::<_, x509_cert::der::Error>((modulus.as_bytes(), exponent.as_bytes()))
    })?;
    reader.finish()?;
    Ok(pair)
}

fn bit_length(be: &[u8]) -> usize {
    match be.first() {
        None => 0,
        Some(first) => (be.len() - 1) * 8 + (8 - first.leading_zeros() as usize),
    }
}

/// The client configuration: ring provider (never the process default), TLS 1.2 and 1.3,
/// `PinVerifier`, no root store, no client certificate, no session resumption, no key log.
fn client_config(unsupported: Arc<AtomicBool>) -> Result<ClientConfig, rustls::Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(PinVerifier {
        provider: Arc::clone(&provider),
        unsupported,
    });
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.resumption = rustls::client::Resumption::disabled();
    Ok(config)
}

/// Upgrades `stream` to TLS and returns the stream and the leaf certificate (DER, as the server
/// sent it). A handshake that fails because of the certificate is `CertificateUnsupported`, any
/// other failure `TlsFailed`; the cause is logged at debug level (rustls errors carry no secrets).
pub(crate) async fn upgrade(
    stream: TcpStream,
    host: &str,
) -> Result<(TlsStream<TcpStream>, Vec<u8>), RdpError> {
    let unsupported = Arc::new(AtomicBool::new(false));
    let config = client_config(Arc::clone(&unsupported)).map_err(|e| {
        log::debug!("RDP TLS configuration failed: {e}");
        RdpError::TlsFailed
    })?;
    let name = ServerName::try_from(host.to_owned()).map_err(|_| RdpError::TlsFailed)?;
    let mut tls = match TlsConnector::from(Arc::new(config))
        .connect(name, stream)
        .await
    {
        Ok(tls) => tls,
        Err(e) => {
            let unsupported = unsupported.load(Ordering::SeqCst);
            log::debug!(
                "RDP TLS handshake with {host} failed (certificate unsupported: {unsupported}): {e}"
            );
            return Err(if unsupported {
                RdpError::CertificateUnsupported
            } else {
                RdpError::TlsFailed
            });
        }
    };
    tls.flush().await.map_err(|_| RdpError::TlsFailed)?;
    let leaf = tls
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certs| certs.first())
        .ok_or(RdpError::TlsFailed)?
        .to_vec();
    Ok((tls, leaf))
}

#[cfg(test)]
mod tests {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{PrivateKeyDer, ServerName, UnixTime};

    use super::*;

    const RSA2048: &[u8] = include_bytes!("../tests/fixtures/rsa2048-sha256.crt");
    const RSA2048_KEY: &[u8] = include_bytes!("../tests/fixtures/rsa2048-sha256.key");
    const RSA2048_SHA1: &[u8] = include_bytes!("../tests/fixtures/rsa2048-sha1.crt");
    const CRITICAL_EXT: &[u8] = include_bytes!("../tests/fixtures/critical-ext.crt");
    const RSA1024: &[u8] = include_bytes!("../tests/fixtures/rsa1024-sha256.crt");
    const EC_P521: &[u8] = include_bytes!("../tests/fixtures/ec-p521.crt");

    fn cert(pem: &[u8]) -> CertificateDer<'static> {
        CertificateDer::from_pem_slice(pem).unwrap()
    }

    fn verifier() -> (PinVerifier, Arc<AtomicBool>) {
        let unsupported = Arc::new(AtomicBool::new(false));
        let verifier = PinVerifier {
            provider: Arc::new(rustls::crypto::ring::default_provider()),
            unsupported: Arc::clone(&unsupported),
        };
        (verifier, unsupported)
    }

    /// A TLS 1.3 `CertificateVerify`-like structure: `scheme` and `signature`.
    fn dss(scheme: SignatureScheme, signature: Vec<u8>) -> DigitallySignedStruct {
        // DigitallySignedStruct has no public constructor; decode its wire form
        let mut wire = Vec::new();
        wire.extend_from_slice(&u16::from(scheme).to_be_bytes());
        wire.extend_from_slice(&u16::try_from(signature.len()).unwrap().to_be_bytes());
        wire.extend_from_slice(&signature);
        let mut reader = rustls::internal::msgs::codec::Reader::init(&wire);
        rustls::internal::msgs::codec::Codec::read(&mut reader).unwrap()
    }

    fn sign(key_pem: &[u8], scheme: SignatureScheme, message: &[u8]) -> Vec<u8> {
        let key = PrivateKeyDer::from_pem_slice(key_pem).unwrap();
        let provider = rustls::crypto::ring::default_provider();
        let key = provider.key_provider.load_private_key(key).unwrap();
        key.choose_scheme(&[scheme]).unwrap().sign(message).unwrap()
    }

    #[test]
    fn the_verifier_needs_no_root_store() {
        // the configuration builds without any root certificate (`client_config` has no root store
        // to fill, so an empty platform store cannot matter) and uses the ring provider
        let config = client_config(Arc::new(AtomicBool::new(false))).unwrap();
        let ring = rustls::crypto::ring::default_provider();
        assert_eq!(
            config.crypto_provider().cipher_suites,
            ring.cipher_suites,
            "the ring provider, not the process default"
        );
        // a self-signed leaf passes, whatever the name and the time
        let (verifier, unsupported) = verifier();
        for pem in [RSA2048, RSA2048_SHA1, CRITICAL_EXT] {
            let name = ServerName::try_from("not-the-certificate-name.example").unwrap();
            let far_future = UnixTime::since_unix_epoch(std::time::Duration::from_secs(1 << 40));
            assert!(
                verifier
                    .verify_server_cert(&cert(pem), &[], &name, &[], far_future)
                    .is_ok()
            );
        }
        assert!(!unsupported.load(Ordering::SeqCst));
    }

    #[test]
    fn a_genuine_handshake_signature_passes_for_tls12_and_tls13() {
        let message = b"handshake transcript";
        let (verifier, unsupported) = verifier();
        let leaf = cert(RSA2048);
        for scheme in [
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PKCS1_SHA256,
        ] {
            let signed = dss(scheme, sign(RSA2048_KEY, scheme, message));
            assert!(
                verifier
                    .verify_tls12_signature(message, &leaf, &signed)
                    .is_ok()
            );
        }
        let pss = SignatureScheme::RSA_PSS_SHA256;
        let signed = dss(pss, sign(RSA2048_KEY, pss, message));
        assert!(
            verifier
                .verify_tls13_signature(message, &leaf, &signed)
                .is_ok()
        );
        assert!(!unsupported.load(Ordering::SeqCst));
    }

    #[test]
    fn a_forged_handshake_signature_fails_and_is_not_called_unsupported() {
        let (verifier, unsupported) = verifier();
        let leaf = cert(RSA2048);
        let pss = SignatureScheme::RSA_PSS_SHA256;
        // signed for another message, and plain garbage
        let wrong = dss(pss, sign(RSA2048_KEY, pss, b"another transcript"));
        let garbage = dss(pss, vec![0x5a; 256]);
        for signed in [&wrong, &garbage] {
            assert_eq!(
                verifier
                    .verify_tls13_signature(b"transcript", &leaf, signed)
                    .err(),
                Some(rustls::Error::InvalidCertificate(
                    CertificateError::BadSignature
                ))
            );
            assert!(
                verifier
                    .verify_tls12_signature(b"transcript", &leaf, signed)
                    .is_err()
            );
        }
        assert!(!unsupported.load(Ordering::SeqCst));
    }

    #[test]
    fn a_certificate_webpki_cannot_parse_is_marked_unsupported() {
        let (verifier, unsupported) = verifier();
        let pss = SignatureScheme::RSA_PSS_SHA256;
        let signed = dss(pss, vec![0x5a; 256]);
        assert!(
            verifier
                .verify_tls13_signature(b"transcript", &cert(CRITICAL_EXT), &signed)
                .is_err()
        );
        assert!(unsupported.load(Ordering::SeqCst));
    }

    #[test]
    fn an_rsa_key_under_2048_bits_is_marked_unsupported() {
        let (verifier, unsupported) = verifier();
        let pss = SignatureScheme::RSA_PSS_SHA256;
        let signed = dss(pss, vec![0x5a; 128]);
        assert_eq!(
            verifier
                .verify_tls12_signature(b"transcript", &cert(RSA1024), &signed)
                .err(),
            Some(rustls::Error::InvalidCertificate(
                CertificateError::BadSignature
            ))
        );
        assert!(unsupported.load(Ordering::SeqCst));
    }

    #[test]
    fn a_signature_of_another_key_type_than_the_certificate_is_not_called_unsupported() {
        // an impostor replays the host's RSA certificate and signs with its own ECDSA key: webpki
        // reports the scheme as unsupported for that key, but the certificate is fine
        let (verifier, unsupported) = verifier();
        let leaf = cert(RSA2048);
        let ecdsa = dss(SignatureScheme::ECDSA_NISTP256_SHA256, vec![0x5a; 72]);
        for result in [
            verifier.verify_tls12_signature(b"transcript", &leaf, &ecdsa),
            verifier.verify_tls13_signature(b"transcript", &leaf, &ecdsa),
        ] {
            assert!(
                matches!(
                    result,
                    Err(rustls::Error::InvalidCertificate(
                        CertificateError::UnsupportedSignatureAlgorithmForPublicKeyContext { .. }
                    ))
                ),
                "{result:?}"
            );
        }
        assert!(!unsupported.load(Ordering::SeqCst));
    }

    #[test]
    fn a_key_type_the_provider_cannot_verify_with_is_marked_unsupported() {
        // ECDSA P-521: the ring provider has no algorithm for that curve
        let (verifier, unsupported) = verifier();
        let signed = dss(SignatureScheme::ECDSA_NISTP384_SHA384, vec![0x5a; 104]);
        assert!(matches!(
            verifier.verify_tls12_signature(b"transcript", &cert(EC_P521), &signed),
            Err(rustls::Error::InvalidCertificate(
                CertificateError::UnsupportedSignatureAlgorithmForPublicKeyContext { .. }
            ))
        ));
        assert!(unsupported.load(Ordering::SeqCst));
    }

    #[test]
    fn key_types_are_compared_with_the_provider_algorithms() {
        let algorithms = rustls::crypto::ring::default_provider().signature_verification_algorithms;
        for (pem, supported) in [(RSA2048, true), (RSA1024, true), (EC_P521, false)] {
            let der = cert(pem);
            let parsed = ParsedCertificate::try_from(&der).unwrap();
            assert_eq!(key_type_supported(&parsed, &algorithms), supported);
        }
    }

    #[test]
    fn rsa_key_sizes_are_read_from_the_certificate() {
        assert!(!rsa_key_out_of_range(&cert(RSA2048)));
        assert!(!rsa_key_out_of_range(&cert(RSA2048_SHA1)));
        assert!(rsa_key_out_of_range(&cert(RSA1024)));
        assert_eq!(bit_length(&[]), 0);
        assert_eq!(bit_length(&[0x01]), 1);
        assert_eq!(bit_length(&[0x80, 0x00]), 16);
    }
}
