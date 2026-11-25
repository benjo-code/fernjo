//! The connect procedure: TCP, X.224 negotiation (CredSSP only), TLS, the certificate decision,
//! credentials, CredSSP and the rest of the RDP handshake. Nothing here keeps the connection alive
//! afterwards; the caller (`session.rs`) takes the framed stream and the connection result.

use std::future::Future;
use std::time::Duration;

use ironrdp_connector::credssp::KerberosConfig;
use ironrdp_connector::{
    BitmapConfig, ClientConnector, ClientConnectorState, ConnectionResult, ConnectorError,
    ConnectorErrorKind, ConnectorResult, Credentials, DesktopSize, Sequence as _,
};
use ironrdp_core::WriteBuf;
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_dvc::DrdynvcClient;
use ironrdp_pdu::gcc::{ConnectionType, KeyboardType};
use ironrdp_pdu::nego::{ConnectionConfirm, FailureCode, SecurityProtocol};
use ironrdp_pdu::rdp::capability_sets::{
    MajorPlatformType, RailSupportLevel, client_codecs_capabilities,
};
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_pdu::x224::X224;
use ironrdp_tokio::{NetworkClient, ShouldUpgrade, TokioFramed};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use x509_cert::der::{Decode as _, Encode as _};

use crate::cert_store::{CertCheck, valid_host};
use crate::text_clipboard::{ClipEvent, LocalText, TextClipboard};
use crate::tls;
use crate::{
    CertChangedDecision, CertChangedPrompt, CertDecision, CertPrompt, ConnectParams, Interaction,
    RdpError,
};

pub(crate) type Framed = TokioFramed<TlsStream<TcpStream>>;

/// The clipboard side channel a connection with the clipboard setting on carries.
pub(crate) struct ClipboardWiring {
    pub events: tokio::sync::mpsc::UnboundedReceiver<ClipEvent>,
    pub local: LocalText,
}

/// A connected, authenticated RDP connection that is ready for the active stage.
pub(crate) struct Established {
    pub framed: Framed,
    pub result: ConnectionResult,
    pub clipboard: Option<ClipboardWiring>,
}

/// Kerberos is not offered: a network request can only come from it, and every login goes through
/// NTLM inside CredSSP.
struct NoNetwork;

impl NetworkClient for NoNetwork {
    async fn send(
        &mut self,
        _request: &ironrdp_connector::sspi::generator::NetworkRequest,
    ) -> ConnectorResult<Vec<u8>> {
        Err(ironrdp_connector::general_err!(
            "network authentication requests are not supported"
        ))
    }
}

/// Runs `step` for at most `limit`; running out of time is `timeout`.
async fn within<T>(limit: Duration, step: impl Future<Output = T>) -> Result<T, RdpError> {
    tokio::time::timeout(limit, step)
        .await
        .map_err(|_| RdpError::Timeout)
}

/// `sha256:<lowercase hex>` of a DER certificate.
pub(crate) fn fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    let mut out = String::with_capacity(7 + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn build_config(
    params: &ConnectParams,
    username: String,
    password: &SecretString,
) -> ironrdp_connector::Config {
    let codecs = client_codecs_capabilities(&[]).unwrap_or_default();
    ironrdp_connector::Config {
        credentials: Credentials::UsernamePassword {
            username,
            password: password.expose_secret().to_string(),
        },
        domain: params.domain.clone(),
        // NLA only: the X.224 request offers CredSSP (HYBRID, HYBRID_EX) and not TLS alone. On the
        // TLS-only path the Client Info PDU carries the password in the clear inside TLS, to
        // whoever holds the certificate the user accepted; with CredSSP a man in the middle gets
        // at most a NetNTLMv2 response. A server whose selection lacks CredSSP (TLS-only, standard
        // RDP security, RDSTLS, ...) is refused with `nla_required` by `check_confirm`, before the
        // TLS handshake.
        enable_tls: false,
        enable_credssp: true,
        keyboard_type: KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: DesktopSize {
            width: params.width,
            height: params.height,
        },
        desktop_scale_factor: 0,
        bitmap: Some(BitmapConfig {
            lossy_compression: true,
            color_depth: u32::from(params.color_depth),
            codecs,
        }),
        client_build: 0,
        client_name: "fernjo".to_string(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_string(),
        #[cfg(windows)]
        platform: MajorPlatformType::WINDOWS,
        #[cfg(target_os = "macos")]
        platform: MajorPlatformType::MACINTOSH,
        #[cfg(not(any(windows, target_os = "macos")))]
        platform: MajorPlatformType::UNIX,
        // the cursor is drawn into the framebuffer by the decoder, so the canvas needs no cursor
        // handling of its own
        enable_server_pointer: true,
        pointer_software_rendering: true,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        compression_type: None,
        multitransport_flags: None,
        performance_flags: PerformanceFlags::default(),
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        hardware_id: None,
        monitor_layout: None,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: None,
        monitor_extended_layout: None,
        relative_mouse_input: false,
        enable_standard_rdp_security: false,
        connection_type: ConnectionType::Lan,
        remote_application_mode: false,
        rail_support_level: RailSupportLevel::empty(),
        enable_audio_capture: false,
        support_dyn_vc_gfx_protocol: false,
    }
}

/// What a connector error means for the user.
pub(crate) fn map_connector_error(error: &ConnectorError) -> RdpError {
    match error.kind() {
        ConnectorErrorKind::Credssp(_) | ConnectorErrorKind::AccessDenied => RdpError::AuthFailed,
        ConnectorErrorKind::Negotiation(_) => RdpError::ConnectFailed,
        ConnectorErrorKind::Encode(_)
        | ConnectorErrorKind::Decode(_)
        | ConnectorErrorKind::Reason(_) => RdpError::Protocol,
        ConnectorErrorKind::General | ConnectorErrorKind::Custom | _ => {
            // look down the source chain: bytes that do not decode are a protocol problem, a
            // connection that went away is not
            let mut sources = Vec::new();
            let mut source = std::error::Error::source(error);
            while let Some(e) = source {
                sources.push(e);
                source = e.source();
            }
            if sources.iter().any(|e| {
                e.downcast_ref::<ironrdp_core::DecodeError>().is_some()
                    || e.downcast_ref::<std::io::Error>().is_some_and(|io| {
                        io.kind() == std::io::ErrorKind::InvalidData
                            // `source()` of an io error skips the error it wraps; look at it directly
                            || io
                                .get_ref()
                                .is_some_and(|inner| inner.is::<ironrdp_core::DecodeError>())
                    })
            }) {
                RdpError::Protocol
            } else if sources
                .iter()
                .any(|e| e.downcast_ref::<std::io::Error>().is_some())
            {
                RdpError::ConnectFailed
            } else {
                RdpError::Protocol
            }
        }
    }
}

/// fernjo's own check of the server's X.224 connection confirm, run before the connector sees it.
/// A server whose selection holds neither `HYBRID` nor `HYBRID_EX` (TLS-only `SSL`, standard RDP
/// security, `RDSTLS`, `RDSAAD`, unknown bits) does not run CredSSP, and one that refuses the
/// CredSSP-only request because it wants TLS-only, standard RDP security or TLS with client
/// certificates does not offer it: `NlaRequired`. IronRDP would refuse a selection outside the
/// request too, but only with a generic error, and fernjo does not rely on that check. Bytes that
/// do not decode and other failure codes are left to the connector.
pub(crate) fn check_confirm(pdu: &[u8]) -> Result<(), RdpError> {
    let Ok(X224(confirm)) = ironrdp_core::decode::<X224<ConnectionConfirm>>(pdu) else {
        return Ok(());
    };
    match confirm {
        ConnectionConfirm::Response { protocol, .. } => {
            if protocol.intersects(SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX) {
                Ok(())
            } else {
                Err(RdpError::NlaRequired)
            }
        }
        ConnectionConfirm::Failure { code } => {
            if code == FailureCode::SSL_REQUIRED_BY_SERVER
                || code == FailureCode::SSL_NOT_ALLOWED_BY_SERVER
                || code == FailureCode::SSL_WITH_USER_AUTH_REQUIRED_BY_SERVER
            {
                Err(RdpError::NlaRequired)
            } else {
                Ok(())
            }
        }
    }
}

/// The X.224 negotiation: `ironrdp_tokio::connect_begin` with `check_confirm` on the server's
/// confirm. A refused confirm ends the attempt before the TLS handshake, so nothing more is sent.
async fn negotiate(
    framed: &mut TokioFramed<TcpStream>,
    connector: &mut ClientConnector,
) -> Result<ShouldUpgrade, RdpError> {
    let mut buf = WriteBuf::new();
    while !connector.should_perform_security_upgrade() {
        if !matches!(
            connector.state,
            ClientConnectorState::ConnectionInitiationWaitConfirm { .. }
        ) {
            ironrdp_tokio::single_sequence_step(framed, connector, &mut buf)
                .await
                .map_err(|e| map_connector_error(&e))?;
            continue;
        }
        let pdu = framed
            .read_by_hint(&ironrdp_pdu::X224_HINT)
            .await
            .map_err(|e| {
                map_connector_error(&ironrdp_connector::custom_err!("read frame by hint", e))
            })?;
        check_confirm(&pdu)?;
        buf.clear();
        let written = connector
            .step(&pdu, framed.last_read_at(), &mut buf)
            .map_err(|e| map_connector_error(&e))?;
        // processing the confirm sends nothing (IronRDP 0.12); a connector that wanted to would be
        // a change this loop does not know, so the attempt ends instead of sending unseen bytes
        if written.size().is_some() {
            return Err(RdpError::Protocol);
        }
    }
    Ok(ironrdp_tokio::skip_connect_begin(connector))
}

/// Runs the whole connect procedure.
pub(crate) async fn establish<I: Interaction>(
    params: &ConnectParams,
    interaction: &I,
) -> Result<Established, RdpError> {
    if !valid_host(&params.host) {
        return Err(RdpError::InvalidHost);
    }
    let limit = params.connect_timeout;

    // 1. network: TCP, the X.224 negotiation and the TLS handshake (bounded together)
    let host = params.host.clone();
    let port = params.port;
    let (mut connector, framed_plain_done) = within(limit, async {
        let stream = TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|_| RdpError::ConnectFailed)?;
        let client_addr = stream.local_addr().map_err(|_| RdpError::ConnectFailed)?;
        let mut framed = TokioFramed::new(stream);
        // the real credentials are put in after the certificate decision
        let placeholder = SecretString::from(String::new());
        let mut connector = ClientConnector::new(
            build_config(params, String::new(), &placeholder),
            client_addr,
        );
        if params.dynamic_resize {
            connector.attach_static_channel(
                DrdynvcClient::new()
                    .with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new()))),
            );
        }
        // a server without NLA is refused here, before TLS, the certificate and the prompts
        let should_upgrade = negotiate(&mut framed, &mut connector).await?;
        let (stream, leftover) = framed.into_inner();
        if !leftover.is_empty() {
            return Err(RdpError::Protocol);
        }
        // RDP servers present self-signed certificates, so chain, name and expiry are not checked
        // and the pin decides after the handshake. The handshake signature is still checked
        // against the certificate's public key (`tls::PinVerifier`), so a server that only
        // replays a public certificate cannot get through.
        let (tls, leaf) = tls::upgrade(stream, &host).await?;
        let cert = x509_cert::Certificate::from_der(&leaf).map_err(|_| RdpError::TlsFailed)?;
        let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
        let der = cert
            .to_der()
            .map_err(|_: x509_cert::der::Error| RdpError::TlsFailed)?;
        let public_key = cert
            .tbs_certificate()
            .subject_public_key_info()
            .subject_public_key
            .as_bytes()
            .ok_or(RdpError::TlsFailed)?
            .to_vec();
        Ok::<_, RdpError>((
            connector,
            (
                TokioFramed::new(tls),
                upgraded,
                fingerprint(&der),
                public_key,
            ),
        ))
    })
    .await??;
    let (mut framed, upgraded, print, public_key) = framed_plain_done;

    // 2. the certificate decision (a person may take as long as they need)
    match params.certs.check(&params.host, params.port, &print)? {
        CertCheck::Match => {}
        CertCheck::Unknown => match interaction
            .certificate(CertPrompt {
                host: params.host.clone(),
                port: params.port,
                fingerprint: print.clone(),
            })
            .await
        {
            CertDecision::TrustOnce => {}
            CertDecision::TrustAlways => params.certs.pin(&params.host, params.port, &print)?,
            CertDecision::Reject => return Err(RdpError::CertificateRejected),
        },
        CertCheck::Changed { old } => match interaction
            .certificate_changed(CertChangedPrompt {
                host: params.host.clone(),
                port: params.port,
                old,
                new: print.clone(),
            })
            .await
        {
            CertChangedDecision::Replace => {
                params.certs.replace(&params.host, params.port, &print)?
            }
            CertChangedDecision::Reject => return Err(RdpError::CertificateRejected),
        },
    }

    // 3. credentials, asked for only now that the server is trusted
    let username = if params.username.is_empty() {
        interaction
            .username(&params.host)
            .await
            .filter(|u| !u.is_empty())
            .ok_or(RdpError::AuthFailed)?
    } else {
        params.username.clone()
    };
    let password = if params.password.expose_secret().is_empty() {
        interaction
            .password(&username, &params.host)
            .await
            .ok_or(RdpError::AuthFailed)?
    } else {
        params.password.clone()
    };
    connector.config.credentials = Credentials::UsernamePassword {
        username,
        password: password.expose_secret().to_string(),
    };

    // 4. clipboard channel, attached before the connection sequence reaches the channel join
    let clipboard = if params.clipboard {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let local: LocalText = Default::default();
        connector.attach_static_channel(ironrdp_cliprdr::Cliprdr::new(Box::new(
            TextClipboard::new(tx, local.clone()),
        )));
        Some(ClipboardWiring { events: rx, local })
    } else {
        None
    };

    // 5. CredSSP and the rest of the handshake (bounded again)
    let server_name = params.host.clone();
    let result = within(limit, async {
        let mut network = NoNetwork;
        ironrdp_tokio::connect_finalize(
            upgraded,
            connector,
            &mut framed,
            &mut network,
            server_name.into(),
            public_key,
            None::<KerberosConfig>,
        )
        .await
        .map_err(|e| map_connector_error(&e))
    })
    .await??
    .into_connected()
    .map_err(|e| map_connector_error(&e))?;

    Ok(Established {
        framed,
        result,
        clipboard,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_pdu::nego::ResponseFlags;

    fn response(protocol: SecurityProtocol) -> Vec<u8> {
        ironrdp_core::encode_vec(&X224(ConnectionConfirm::Response {
            flags: ResponseFlags::empty(),
            protocol,
        }))
        .unwrap()
    }

    fn failure(code: FailureCode) -> Vec<u8> {
        ironrdp_core::encode_vec(&X224(ConnectionConfirm::Failure { code })).unwrap()
    }

    #[test]
    fn credssp_passes_and_every_selection_without_it_is_nla_required() {
        for ok in [
            SecurityProtocol::HYBRID,
            SecurityProtocol::HYBRID_EX,
            SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX,
            SecurityProtocol::SSL | SecurityProtocol::HYBRID,
        ] {
            assert_eq!(check_confirm(&response(ok)), Ok(()), "{ok:?}");
        }
        let unknown_bit = SecurityProtocol::from_bits_retain(0x0000_0100);
        for refused in [
            SecurityProtocol::SSL,
            SecurityProtocol::empty(),
            SecurityProtocol::RDSTLS,
            SecurityProtocol::RDSAAD,
            SecurityProtocol::SSL | SecurityProtocol::RDSTLS,
            SecurityProtocol::SSL | unknown_bit,
            unknown_bit,
        ] {
            assert_eq!(
                check_confirm(&response(refused)),
                Err(RdpError::NlaRequired),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn only_failures_that_mean_no_nla_are_nla_required() {
        for code in [
            FailureCode::SSL_REQUIRED_BY_SERVER,
            FailureCode::SSL_NOT_ALLOWED_BY_SERVER,
            FailureCode::SSL_WITH_USER_AUTH_REQUIRED_BY_SERVER,
        ] {
            assert_eq!(check_confirm(&failure(code)), Err(RdpError::NlaRequired));
        }
        // left to the connector, which reports them as before
        for code in [
            FailureCode::SSL_CERT_NOT_ON_SERVER,
            FailureCode::INCONSISTENT_FLAGS,
            FailureCode::HYBRID_REQUIRED_BY_SERVER,
        ] {
            assert_eq!(check_confirm(&failure(code)), Ok(()));
        }
    }

    #[test]
    fn bytes_that_do_not_decode_are_left_to_the_connector() {
        assert_eq!(check_confirm(&[0x01, 0x00, 0x00, 0x08, 0xde, 0xad]), Ok(()));
    }
}
