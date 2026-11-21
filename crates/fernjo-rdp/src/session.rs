//! A running RDP session: one task owns the connection, everything else talks to it through a
//! command queue and an event queue.

use std::sync::Arc;
use std::time::Duration;

use ironrdp_cliprdr::CliprdrClient;
use ironrdp_cliprdr::backend::ClipboardMessage;
use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId};
use ironrdp_connector::connection_activation::ConnectionActivationState;
use ironrdp_displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_input::{Database, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput, fast_path};
use ironrdp_tokio::{FramedWrite as _, TokioFramed, single_sequence_step_read, split_tokio_framed};
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};

use crate::connect::{ClipboardWiring, Established, establish};
use crate::text_clipboard::{ClipEvent, LocalText};
use crate::{ConnectParams, Framebuffer, InputEvent, Interaction, MouseButton, RdpError, Rect};

/// Events waiting for the consumer; `Damage` is merged beyond this.
const EVENT_QUEUE: usize = 8;
/// Commands waiting for the connection task.
const COMMAND_QUEUE: usize = 64;
/// How long one step of a reactivation may wait for the server.
const REACTIVATION_STEP_LIMIT: Duration = Duration::from_secs(30);
/// How long a final event may wait for a consumer that has stopped reading.
const DELIVER_LIMIT: Duration = Duration::from_secs(5);

type Reader = TokioFramed<ReadHalf<ironrdp_tls::TlsStream<TcpStream>>>;
type Writer = TokioFramed<WriteHalf<ironrdp_tls::TlsStream<TcpStream>>>;

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// The server ended the session (logoff, disconnect, shutdown).
    ServerClosed,
    /// The connection broke or the server sent something the client could not follow.
    ConnectionLost,
    /// [`Session::close`] was called or the session was dropped.
    ClosedByUser,
}

/// What a session reports. The queue is bounded; `Damage` events are merged while it is full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// This part of the [`Framebuffer`] changed.
    Damage(Rect),
    /// The desktop has a new size; the framebuffer was resized and is blank until the next damage.
    DesktopSize { width: u16, height: u16 },
    /// The remote side copied this text (line ends already converted, size capped).
    Clipboard(String),
    /// The last event of a session.
    Closed(CloseReason),
}

enum Command {
    Input(Vec<InputEvent>),
    Resize(u16, u16),
    Clipboard(String),
}

/// Handle on a running session. Dropping it ends the session.
pub struct Session {
    framebuffer: Framebuffer,
    commands: mpsc::Sender<Command>,
    closing: Arc<Notify>,
    clipboard: bool,
}

impl Session {
    /// Connects, authenticates and starts the session task.
    ///
    /// The certificate decision comes before any credential is sent; the network steps are bound
    /// by `params.connect_timeout`, the time spent on prompts is not.
    ///
    /// # Errors
    /// See [`RdpError`]; a server that announces a zero or oversize desktop is `desktop_too_large`.
    pub async fn connect<I: Interaction>(
        params: ConnectParams,
        interaction: Arc<I>,
    ) -> Result<(Session, mpsc::Receiver<SessionEvent>), RdpError> {
        let established = establish(&params, &*interaction).await?;
        let size = established.result.desktop_size;
        let framebuffer = Framebuffer::new(size.width, size.height)?;
        let (events_tx, events_rx) = mpsc::channel(EVENT_QUEUE);
        let (commands_tx, commands_rx) = mpsc::channel(COMMAND_QUEUE);
        let closing = Arc::new(Notify::new());
        let clipboard = established.clipboard.is_some();
        tokio::spawn(run(
            established,
            framebuffer.clone(),
            events_tx,
            commands_rx,
            closing.clone(),
        ));
        Ok((
            Session {
                framebuffer,
                commands: commands_tx,
                closing,
                clipboard,
            },
            events_rx,
        ))
    }

    /// The pixels of the desktop; shared with the session task.
    pub fn framebuffer(&self) -> Framebuffer {
        self.framebuffer.clone()
    }

    /// Sends keyboard and mouse events. Mouse coordinates are clamped to the desktop.
    ///
    /// # Errors
    /// `lost` when the session has ended.
    pub async fn input(&self, events: Vec<InputEvent>) -> Result<(), RdpError> {
        self.send(Command::Input(events)).await
    }

    /// Asks the server for a new desktop size (display control). A server without it keeps its
    /// size and this still succeeds.
    ///
    /// # Errors
    /// `lost` when the session has ended.
    pub async fn resize(&self, width: u16, height: u16) -> Result<(), RdpError> {
        self.send(Command::Resize(width, height)).await
    }

    /// Offers `text` to the remote clipboard. Nothing happens when the clipboard is off for this
    /// session, when the text is over the cap, or when it is the text the remote side just sent.
    ///
    /// # Errors
    /// `lost` when the session has ended.
    pub async fn set_clipboard_text(&self, text: String) -> Result<(), RdpError> {
        if !self.clipboard {
            return Ok(());
        }
        self.send(Command::Clipboard(text)).await
    }

    /// Ends the session; the event queue then delivers `Closed(ClosedByUser)`.
    pub fn close(&self) {
        self.closing.notify_one();
    }

    async fn send(&self, command: Command) -> Result<(), RdpError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| RdpError::Lost)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.closing.notify_one();
    }
}

// ---- the session task -------------------------------------------------------------------------

/// Hands events to the consumer; `Damage` is merged while the queue is full.
struct Emitter {
    tx: mpsc::Sender<SessionEvent>,
    pending: Option<Rect>,
}

impl Emitter {
    fn damage(&mut self, rect: Rect) {
        let merged = self.pending.map_or(rect, |p| p.union(rect));
        match self.tx.try_reserve() {
            Ok(permit) => {
                permit.send(SessionEvent::Damage(merged));
                self.pending = None;
            }
            Err(_) => self.pending = Some(merged),
        }
    }

    /// Delivers an event that must not be dropped; waits for room, but not forever.
    async fn deliver(&self, event: SessionEvent) {
        let _ = tokio::time::timeout(DELIVER_LIMIT, self.tx.send(event)).await;
    }
}

/// How the main loop ended.
type Exit = CloseReason;

async fn run(
    established: Established,
    framebuffer: Framebuffer,
    events: mpsc::Sender<SessionEvent>,
    commands: mpsc::Receiver<Command>,
    closing: Arc<Notify>,
) {
    let mut emitter = Emitter {
        tx: events,
        pending: None,
    };
    let reason = drive(established, &framebuffer, &mut emitter, commands, closing).await;
    emitter.deliver(SessionEvent::Closed(reason)).await;
}

async fn drive(
    established: Established,
    framebuffer: &Framebuffer,
    emitter: &mut Emitter,
    mut commands: mpsc::Receiver<Command>,
    closing: Arc<Notify>,
) -> Exit {
    let Established {
        framed,
        result,
        clipboard,
    } = established;
    let (mut reader, mut writer): (Reader, Writer) = split_tokio_framed(framed);
    let size = result.desktop_size;
    let mut image = DecodedImage::new(PixelFormat::RgbA32, size.width, size.height);
    let activation_factory = result.activation_factory;
    let mut stage = ActiveStageBuilder {
        static_channels: result.static_channels,
        user_channel_id: result.user_channel_id,
        io_channel_id: result.io_channel_id,
        message_channel_id: result.message_channel_id,
        share_id: result.share_id,
        compression_type: result.compression_type,
        enable_server_pointer: result.enable_server_pointer,
        pointer_software_rendering: result.pointer_software_rendering,
    }
    .build();
    let (mut clip_rx, local_text): (
        Option<mpsc::UnboundedReceiver<ClipEvent>>,
        Option<LocalText>,
    ) = match clipboard {
        Some(ClipboardWiring { events, local }) => (Some(events), Some(local)),
        None => (None, None),
    };
    let mut keys = Database::new();
    let mut last_remote_text: Option<String> = None;

    loop {
        let reserve_tx = emitter.tx.clone();
        let outputs: Vec<ActiveStageOutput> = tokio::select! {
            frame = reader.read_pdu() => {
                let Ok((action, payload)) = frame else { return CloseReason::ConnectionLost };
                match stage.process(&mut image, action, &payload, None) {
                    Ok(outputs) => outputs,
                    Err(_) => return CloseReason::ConnectionLost,
                }
            }
            command = commands.recv() => {
                let Some(command) = command else { return CloseReason::ClosedByUser };
                match command {
                    Command::Input(events) => {
                        let (w, h) = framebuffer.size();
                        let fast = to_fast_path(&mut keys, events, w, h);
                        match stage.process_fastpath_input(&mut image, &fast) {
                            Ok(outputs) => outputs,
                            Err(_) => return CloseReason::ConnectionLost,
                        }
                    }
                    Command::Resize(width, height) => {
                        let (width, height) =
                            MonitorLayoutEntry::adjust_display_size(u32::from(width), u32::from(height));
                        match stage.encode_resize(width, height, None, None) {
                            Some(Ok(frame)) => vec![ActiveStageOutput::ResponseFrame(frame)],
                            Some(Err(_)) => return CloseReason::ConnectionLost,
                            // no display control channel: the desktop keeps its size
                            None => Vec::new(),
                        }
                    }
                    Command::Clipboard(text) => {
                        if last_remote_text.as_deref() == Some(text.as_str()) {
                            Vec::new()
                        } else if let Some(local) = &local_text {
                            // not announced when it is over the cap: the backend could not serve it
                            if text.len() <= crate::MAX_CLIPBOARD_BYTES {
                                if let Ok(mut slot) = local.lock() {
                                    *slot = Some(text);
                                }
                                let message = ClipboardMessage::SendInitiateCopy(vec![
                                    ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
                                ]);
                                match clipboard_frame(&mut stage, message) {
                                    Ok(Some(frame)) => vec![ActiveStageOutput::ResponseFrame(frame)],
                                    Ok(None) => Vec::new(),
                                    Err(()) => return CloseReason::ConnectionLost,
                                }
                            } else {
                                Vec::new()
                            }
                        } else {
                            Vec::new()
                        }
                    }
                }
            }
            event = async {
                match clip_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match event {
                    Some(ClipEvent::Message(message)) => match clipboard_frame(&mut stage, message) {
                        Ok(Some(frame)) => vec![ActiveStageOutput::ResponseFrame(frame)],
                        Ok(None) => Vec::new(),
                        Err(()) => return CloseReason::ConnectionLost,
                    },
                    Some(ClipEvent::RemoteText(text)) => {
                        last_remote_text = Some(text.clone());
                        emitter.deliver(SessionEvent::Clipboard(text)).await;
                        Vec::new()
                    }
                    None => {
                        clip_rx = None;
                        Vec::new()
                    }
                }
            }
            permit = reserve_tx.reserve(), if emitter.pending.is_some() => {
                if let (Ok(permit), Some(rect)) = (permit, emitter.pending.take()) {
                    permit.send(SessionEvent::Damage(rect));
                }
                Vec::new()
            }
            () = closing.notified() => {
                if let Ok(outputs) = stage.graceful_shutdown() {
                    for output in outputs {
                        if let ActiveStageOutput::ResponseFrame(frame) = output {
                            let _ = writer.write_all(&frame).await;
                        }
                    }
                }
                return CloseReason::ClosedByUser;
            }
        };

        for output in outputs {
            match output {
                ActiveStageOutput::ResponseFrame(frame) => {
                    if frame.is_empty() {
                        continue;
                    }
                    if writer.write_all(&frame).await.is_err() {
                        return CloseReason::ConnectionLost;
                    }
                }
                ActiveStageOutput::GraphicsUpdate(region) => {
                    if let Some(rect) = copy_region(&image, &region, framebuffer) {
                        emitter.damage(rect);
                    }
                }
                ActiveStageOutput::Terminate(_) => return CloseReason::ServerClosed,
                ActiveStageOutput::DeactivateAll => {
                    // Deactivation-Reactivation: the server renegotiates (typically after a resize)
                    let mut activation = activation_factory.create();
                    let mut buf = ironrdp_core::WriteBuf::new();
                    let finalized = loop {
                        // a server that goes quiet here must neither hold the session open nor
                        // make it impossible to close
                        let read =
                            single_sequence_step_read(&mut reader, &mut activation, &mut buf);
                        let step = tokio::select! {
                            read = tokio::time::timeout(REACTIVATION_STEP_LIMIT, read) => match read {
                                Ok(Ok(step)) => step,
                                _ => return CloseReason::ConnectionLost,
                            },
                            () = closing.notified() => return CloseReason::ClosedByUser,
                        };
                        if step.size().is_some() && writer.write_all(buf.filled()).await.is_err() {
                            return CloseReason::ConnectionLost;
                        }
                        if let ConnectionActivationState::Finalized {
                            desktop_size,
                            share_id,
                            enable_server_pointer,
                            pointer_software_rendering,
                            ..
                        } = activation.connection_activation_state()
                        {
                            break (
                                desktop_size,
                                share_id,
                                enable_server_pointer,
                                pointer_software_rendering,
                            );
                        }
                    };
                    let (desktop, share_id, server_pointer, software_pointer) = finalized;
                    // a size the framebuffer refuses ends the session instead of allocating it
                    if framebuffer.resize(desktop.width, desktop.height).is_err() {
                        return CloseReason::ConnectionLost;
                    }
                    image = DecodedImage::new(PixelFormat::RgbA32, desktop.width, desktop.height);
                    stage.set_fastpath_processor(
                        fast_path::ProcessorBuilder {
                            io_channel_id: activation.io_channel_id(),
                            user_channel_id: activation.user_channel_id(),
                            share_id,
                            enable_server_pointer: server_pointer,
                            pointer_software_rendering: software_pointer,
                        }
                        .build(),
                    );
                    stage.set_share_id(share_id);
                    stage.set_enable_server_pointer(server_pointer);
                    emitter.pending = None;
                    emitter
                        .deliver(SessionEvent::DesktopSize {
                            width: desktop.width,
                            height: desktop.height,
                        })
                        .await;
                }
                // the cursor is drawn into the image; the rest has no consumer here
                _ => {}
            }
        }
    }
}

/// Encodes a CLIPRDR message as a frame for the wire, or `None` when it is not one we handle.
fn clipboard_frame(
    stage: &mut ActiveStage,
    message: ClipboardMessage,
) -> Result<Option<Vec<u8>>, ()> {
    let Some(cliprdr) = stage.get_svc_processor_mut::<CliprdrClient>() else {
        return Ok(None);
    };
    let messages = match message {
        ClipboardMessage::SendInitiateCopy(formats) => cliprdr.initiate_copy(&formats),
        ClipboardMessage::SendFormatData(response) => cliprdr.submit_format_data(response),
        ClipboardMessage::SendInitiatePaste(format) => cliprdr.initiate_paste(format),
        // files are not shared; anything else the backend reports is dropped
        _ => return Ok(None),
    }
    .map_err(|_| ())?;
    stage
        .process_svc_processor_messages(messages)
        .map(Some)
        .map_err(|_| ())
}

/// Copies the changed part of the decoded image into the shared framebuffer and returns it.
fn copy_region(
    image: &DecodedImage,
    region: &InclusiveRectangle,
    framebuffer: &Framebuffer,
) -> Option<Rect> {
    let rect = Rect {
        x: region.left,
        y: region.top,
        width: region.right.checked_sub(region.left)?.checked_add(1)?,
        height: region.bottom.checked_sub(region.top)?.checked_add(1)?,
    }
    .clamp_to(image.width(), image.height())?;
    let stride = image.stride();
    let data = image.data();
    let row_bytes = usize::from(rect.width) * 4;
    let mut packed = Vec::with_capacity(row_bytes * usize::from(rect.height));
    for row in 0..usize::from(rect.height) {
        let from = (usize::from(rect.y) + row) * stride + usize::from(rect.x) * 4;
        packed.extend_from_slice(data.get(from..from + row_bytes)?);
    }
    framebuffer.write_region(rect, &packed);
    Some(rect)
}

fn to_fast_path(
    keys: &mut Database,
    events: Vec<InputEvent>,
    width: u16,
    height: u16,
) -> Vec<FastPathInputEvent> {
    let mut out = Vec::new();
    for event in events {
        let operation = match event {
            InputEvent::Key {
                scancode,
                extended,
                pressed,
            } => {
                let code = Scancode::from_u8(extended, scancode);
                if pressed {
                    Operation::KeyPressed(code)
                } else {
                    Operation::KeyReleased(code)
                }
            }
            InputEvent::MouseMove { x, y } => Operation::MouseMove(MousePosition {
                x: x.min(width.saturating_sub(1)),
                y: y.min(height.saturating_sub(1)),
            }),
            InputEvent::MouseButton { button, pressed } => {
                let button = match button {
                    MouseButton::Left => ironrdp_input::MouseButton::Left,
                    MouseButton::Middle => ironrdp_input::MouseButton::Middle,
                    MouseButton::Right => ironrdp_input::MouseButton::Right,
                    MouseButton::X1 => ironrdp_input::MouseButton::X1,
                    MouseButton::X2 => ironrdp_input::MouseButton::X2,
                };
                if pressed {
                    Operation::MouseButtonPressed(button)
                } else {
                    Operation::MouseButtonReleased(button)
                }
            }
            InputEvent::Wheel { vertical, delta } => Operation::WheelRotations(WheelRotations {
                is_vertical: vertical,
                rotation_units: delta,
            }),
        };
        out.extend(keys.apply([operation]));
    }
    out
}

#[cfg(test)]
mod tests;
