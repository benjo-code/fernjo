//! RDP sessions inside the session manager: the connect task, the prompts and the forwarder that
//! turns damage events into region messages for the canvas.
//!
//! A child of `sessions` so it shares the manager's private types (`Shared`, `Op`, `Entry`) with
//! the SSH path instead of widening their visibility. The wire format of a region message (the raw
//! message of an RDP session) is `u16 x, u16 y, u16 width, u16 height` (little endian) followed by
//! `width * height * 4` RGBA bytes; see the contract at the top of `sessions.rs`.

use std::sync::Arc;

use fernjo_core::RdpResolution;
use fernjo_rdp::{
    CertChangedDecision, CertChangedPrompt, CertDecision, CertPrompt, CertStore, CloseReason,
    ConnectParams, Framebuffer, InputEvent, Interaction, MAX_DESKTOP_DIMENSION, MAX_DESKTOP_PIXELS,
    MAX_INPUT_EVENTS, Rect, Session, SessionEvent,
};
use secrecy::SecretString;
use tokio::sync::mpsc::{Receiver, UnboundedReceiver, unbounded_channel};

use super::{
    ConnState, Ended, Entry, HostKeyAnswer, HostKeyChangedAnswer, Op, Phase, PromptAnswer,
    PromptView, SessionManager, Shared, TauriInteraction, UiEvent, error, install_ops,
    release_unhandled, take_resize,
};
use crate::commands::CommandError;
use crate::connect_plan::{RdpCredentialPlan, RdpPlan};
use crate::session_flow::FlowControl;

/// Region messages the UI may be behind by, measured in whole-desktop messages: the spike
/// (`Docs/DECISIONS.md`, "M8: RDP render path spike") needs three on Windows for 30 painted frames
/// per second while scrolling; Linux and macOS pass with one. The bound is applied in bytes, see
/// [`credit_for`], so small messages do not use it up.
pub(crate) const RDP_FRAMES_IN_FLIGHT: usize = 3;

/// The desktop size asked for when the UI could not say how big its tab is.
const DEFAULT_SIZE: (u16, u16) = (1280, 800);
/// Smallest desktop a resize or a fit-to-tab connect asks for.
const MIN_DIMENSION: u32 = 200;

/// Bytes of a region message header.
pub(crate) const HEADER_LEN: usize = 8;

/// A desktop size the server can be asked for: both sides clamped to 200..=8192, the area scaled
/// down (keeping the aspect) to at most [`MAX_DESKTOP_PIXELS`], and each side rounded down to an
/// even number (some servers refuse an odd width). `None` for a zero side, which a hidden or
/// collapsed tab reports and which is never sent.
pub(crate) fn desktop_size(width: u32, height: u32) -> Option<(u16, u16)> {
    if width == 0 || height == 0 {
        return None;
    }
    let clamp = |v: u32| v.clamp(MIN_DIMENSION, u32::from(MAX_DESKTOP_DIMENSION));
    let (mut w, mut h) = (clamp(width), clamp(height));
    let limit = u64::from(MAX_DESKTOP_PIXELS);
    if u64::from(w) * u64::from(h) > limit {
        // both sides are in 200..=8192 here, so the area is finite and the factor in (0, 1)
        let scale = (limit as f64 / (f64::from(w) * f64::from(h))).sqrt();
        w = clamp((f64::from(w) * scale) as u32);
        h = clamp((f64::from(h) * scale) as u32);
    }
    let even = |v: u32| u16::try_from(v & !1).unwrap_or(MAX_DESKTOP_DIMENSION);
    let (mut w, mut h) = (even(w), even(h));
    // the float rounding must never leave the area above the limit
    while u64::from(w) * u64::from(h) > limit {
        if w >= h {
            w -= 2;
        } else {
            h -= 2;
        }
    }
    Some((w, h))
}

/// The credit for region messages of a desktop of this size: sending stops once more than
/// [`RDP_FRAMES_IN_FLIGHT`] minus one whole frames are unacknowledged (so the last one sent may
/// make it three) and resumes as soon as an acknowledgement brings the count back to that level,
/// which keeps the pipeline full instead of draining it before each refill.
pub(crate) fn credit_for(width: u16, height: u16) -> FlowControl {
    let frame = usize::from(width) * usize::from(height) * 4;
    let high = frame * (RDP_FRAMES_IN_FLIGHT - 1);
    FlowControl::new(high, high)
}

/// The union of the damage that has not been sent yet.
#[derive(Debug, Default)]
pub(crate) struct Damage(Option<Rect>);

impl Damage {
    pub(crate) fn add(&mut self, rect: Rect) {
        self.0 = Some(match self.0 {
            Some(union) => union.union(rect),
            None => rect,
        });
    }

    pub(crate) fn take(&mut self) -> Option<Rect> {
        self.0.take()
    }

    pub(crate) fn clear(&mut self) {
        self.0 = None;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_none()
    }
}

/// The raw message for `rect` of the desktop: header and the pixels as they are now (the latest
/// pixels win). `None` when the rectangle lies outside the desktop or the desktop changed size
/// while it was copied (a `DesktopSize` event follows then).
pub(crate) fn region_message(framebuffer: &Framebuffer, rect: Rect) -> Option<Vec<u8>> {
    let (width, height) = framebuffer.size();
    let clipped = rect.clamp_to(width, height)?;
    let pixels = framebuffer.copy_region(clipped);
    if pixels.len() != usize::from(clipped.width) * usize::from(clipped.height) * 4 {
        return None;
    }
    let mut message = Vec::with_capacity(HEADER_LEN + pixels.len());
    for field in [clipped.x, clipped.y, clipped.width, clipped.height] {
        message.extend_from_slice(&field.to_le_bytes());
    }
    message.extend_from_slice(&pixels);
    Some(message)
}

/// How an RDP session ended. Never clean: there is no shell exit status, and the tab of a
/// remote desktop is never closed behind the user's back.
pub(crate) fn classify_rdp_end(reason: CloseReason) -> Ended {
    let text = match reason {
        CloseReason::ServerClosed => "The remote desktop session ended.",
        CloseReason::ConnectionLost => "The connection was lost.",
        CloseReason::ClosedByUser => "The session was closed.",
        CloseReason::ChannelTooLarge => "The server sent a channel message that is too large.",
        CloseReason::UpdateOutOfLimits => {
            "The server sent a display update beyond the limits of this client."
        }
    };
    Ended {
        text: text.to_string(),
        exit_status: None,
        clean: false,
    }
}

/// Answers the RDP layer's questions through prompt events. A closed hub, a cancel or an answer
/// of the wrong shape is the safe answer (reject, nothing typed).
pub(crate) struct RdpInteraction(pub(crate) TauriInteraction);

impl Interaction for RdpInteraction {
    async fn certificate(&self, prompt: CertPrompt) -> CertDecision {
        let view = PromptView::Certificate {
            host: prompt.host,
            port: prompt.port,
            fingerprint: prompt.fingerprint,
        };
        match self.0.ask(view).await {
            Some(PromptAnswer::HostKey { decision }) => match decision {
                HostKeyAnswer::Once => CertDecision::TrustOnce,
                HostKeyAnswer::Always => CertDecision::TrustAlways,
                HostKeyAnswer::Reject => CertDecision::Reject,
            },
            _ => CertDecision::Reject,
        }
    }

    async fn certificate_changed(&self, prompt: CertChangedPrompt) -> CertChangedDecision {
        let view = PromptView::CertificateChanged {
            host: prompt.host,
            port: prompt.port,
            old: prompt.old,
            new: prompt.new,
        };
        match self.0.ask(view).await {
            Some(PromptAnswer::HostKeyChanged {
                decision: HostKeyChangedAnswer::Replace,
            }) => CertChangedDecision::Replace,
            _ => CertChangedDecision::Reject,
        }
    }

    async fn username(&self, host: &str) -> Option<String> {
        fernjo_ssh::Interaction::username(&self.0, host).await
    }

    async fn password(&self, user: &str, host: &str) -> Option<SecretString> {
        fernjo_ssh::Interaction::password(&self.0, user, host).await
    }
}

/// The connection parameters of a plan. `size` is what the UI reported (pixels of the tab); it
/// matters only for a fit-to-tab connection. A fixed resolution turns the display control channel
/// off, so the desktop never follows the tab.
pub(crate) fn build_params(plan: RdpPlan, size: (u32, u32), certs: CertStore) -> ConnectParams {
    let password = match plan.credential {
        RdpCredentialPlan::Ready(password) => password,
        _ => SecretString::from(String::new()),
    };
    let mut params = ConnectParams::new(
        plan.host,
        plan.port,
        plan.username.unwrap_or_default(),
        password,
    );
    // A user name typed as `DOMAIN\user` already names its domain, and the login layer refuses
    // that form together with a separate domain, so the configured domain is not passed then.
    params.domain = plan.domain.filter(|_| !params.username.contains('\\'));
    params.color_depth = plan.color_depth;
    params.clipboard = plan.clipboard;
    params.certs = certs;
    let (width, height) = match plan.resolution {
        RdpResolution::Fit => desktop_size(size.0, size.1).unwrap_or(DEFAULT_SIZE),
        RdpResolution::Fixed { width, height } => {
            desktop_size(u32::from(width), u32::from(height)).unwrap_or(DEFAULT_SIZE)
        }
    };
    params.width = width;
    params.height = height;
    params.dynamic_resize = plan.resolution == RdpResolution::Fit;
    params
}

/// Starts (or restarts) the connect task for an RDP plan, or parks the session until the vault
/// is unlocked.
pub(super) fn start(entry: &Arc<Entry>, plan: RdpPlan, certs: CertStore) {
    let shared = &entry.shared;
    let note = match &plan.credential {
        RdpCredentialPlan::NeedsVault => {
            shared.set_phase(Phase::NeedsVault);
            shared.sink.state(ConnState::NeedsVault, None);
            return;
        }
        RdpCredentialPlan::Missing(reason) => Some(reason.text().to_string()),
        RdpCredentialPlan::None | RdpCredentialPlan::Ready(_) => None,
    };
    shared.set_phase(Phase::Connecting);
    shared.sink.state(ConnState::Connecting, note);
    let size = shared.size.lock().map_or((0, 0), |s| *s);
    let params = build_params(plan, size, certs);
    let handle = tauri::async_runtime::spawn(run(Arc::clone(shared), params));
    if let Ok(mut task) = entry.task.lock() {
        *task = Some(handle);
    }
}

/// One connection attempt and, once connected, the region forwarder.
async fn run(shared: Arc<Shared>, params: ConnectParams) {
    let interaction = Arc::new(RdpInteraction(TauriInteraction {
        hub: Arc::clone(&shared.hub),
        sink: Arc::clone(&shared.sink),
    }));
    let started = (params.width, params.height);
    let dynamic = params.dynamic_resize;
    let (session, rx) = match Session::connect(params, interaction).await {
        Ok(connected) => connected,
        Err(e) => {
            shared.failed(e.to_string());
            return;
        }
    };
    let framebuffer = session.framebuffer();
    let (tx, ops) = unbounded_channel();
    // A resize that arrived while connecting is applied now.
    install_ops(&shared, tx, |size| {
        dynamic && desktop_size(size.0, size.1).is_some_and(|wanted| wanted != started)
    });
    tauri::async_runtime::spawn(write_ops(session, ops, Arc::clone(&shared)));
    let (width, height) = framebuffer.size();
    if let Ok(mut flow) = shared.flow.lock() {
        *flow = credit_for(width, height);
    }
    shared.set_phase(Phase::Connected);
    shared.sink.state(ConnState::Connected, None);
    let ended = forward(&shared, rx, framebuffer).await;
    if let Ok(mut slot) = shared.ops.lock() {
        // Dropping the sender ends the writer, which drops (and so closes) the session.
        *slot = None;
    }
    shared.disconnected(ended);
}

/// Applies input, resizes, clipboard text and the close request to the session, in the order they
/// were queued.
async fn write_ops(session: Session, mut ops: UnboundedReceiver<Op>, shared: Arc<Shared>) {
    while let Some(op) = ops.recv().await {
        let weight = op.weight();
        let stop = match op {
            Op::Rdp(events) => session.input(events).await.is_err(),
            Op::Clipboard(text) => session.set_clipboard_text(text).await.is_err(),
            Op::Resize => {
                let (width, height) = take_resize(&shared);
                if let Some((width, height)) = desktop_size(width, height) {
                    let _ = session.resize(width, height).await;
                }
                false
            }
            // SSH only
            Op::Data(_) => false,
            Op::Close => {
                session.close();
                true
            }
        };
        shared.input_budget.release(weight);
        if stop {
            break;
        }
    }
    release_unhandled(&mut ops, &shared);
}

/// Sends the pending damage as one region message once the flow control has credit; until then
/// the events keep being read and merged, so the newest pixels are what goes out and the queue
/// towards the session never backs up. Ends with the session.
pub(crate) async fn forward(
    shared: &Shared,
    mut rx: Receiver<SessionEvent>,
    framebuffer: Framebuffer,
) -> Ended {
    // The canvas needs the size before the first region.
    let (width, height) = framebuffer.size();
    shared.sink.event(&UiEvent::Desktop { width, height });
    let mut damage = Damage::default();
    loop {
        if !damage.is_empty() && shared.flow.lock().map_or(true, |f| f.should_send()) {
            if let Some(rect) = damage.take()
                && let Some(message) = region_message(&framebuffer, rect)
            {
                if let Ok(mut flow) = shared.flow.lock() {
                    flow.on_sent(message.len());
                }
                shared.sink.output(message);
            }
            continue;
        }
        let event = if damage.is_empty() {
            rx.recv().await
        } else {
            // out of credit: an acknowledgement wakes this up, a damage event is merged
            tokio::select! {
                event = rx.recv() => event,
                () = shared.wake.notified() => continue,
            }
        };
        match event {
            Some(SessionEvent::Damage(rect)) => damage.add(rect),
            Some(SessionEvent::DesktopSize { width, height }) => {
                // what was pending belongs to the old desktop
                damage.clear();
                if let Ok(mut flow) = shared.flow.lock() {
                    *flow = credit_for(width, height);
                }
                shared.sink.event(&UiEvent::Desktop { width, height });
            }
            Some(SessionEvent::Clipboard(text)) => {
                shared.sink.event(&UiEvent::Clipboard { text });
            }
            Some(SessionEvent::Closed(reason)) => return classify_rdp_end(reason),
            None => return classify_rdp_end(CloseReason::ConnectionLost),
        }
    }
}

impl SessionManager {
    /// Queues keyboard and mouse events for the session.
    ///
    /// # Errors
    /// `too_many_events` above [`MAX_INPUT_EVENTS`]; `not_found`; `session_not_connected` while
    /// there is no live session. Mouse coordinates are clamped to the desktop by the session.
    pub fn rdp_input(&self, id: &str, events: Vec<InputEvent>) -> Result<(), CommandError> {
        if events.len() > MAX_INPUT_EVENTS {
            return Err(error(
                "too_many_events",
                "too many input events in one call",
            ));
        }
        self.send_op(id, Op::Rdp(events))
    }

    /// Offers the UI's clipboard text to the remote side (the session ignores it when the
    /// clipboard is off, when it is over the cap, or when it is what the remote side just sent).
    ///
    /// # Errors
    /// `not_found`; `session_not_connected` while there is no live session.
    pub fn rdp_clipboard(&self, id: &str, text: String) -> Result<(), CommandError> {
        // over the cap nothing would be sent anyway; do not queue megabytes for the session task
        if text.len() > fernjo_rdp::MAX_CLIPBOARD_BYTES {
            return Ok(());
        }
        self.send_op(id, Op::Clipboard(text))
    }

    fn send_op(&self, id: &str, op: Op) -> Result<(), CommandError> {
        let entry = self.entry(id)?;
        entry.shared.queue(op, || {
            error("session_not_connected", "the session is not connected")
        })
    }
}

#[cfg(test)]
mod tests;
