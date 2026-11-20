//! The text-only CLIPRDR backend: the one place that speaks the clipboard protocol for fernjo.
//!
//! Local text is announced as `CF_UNICODETEXT` and served when the server asks for it; remote text
//! is requested as soon as the server announces it and handed to the session as an event. Files,
//! images and every other format are ignored.

use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
    OwnedFormatDataResponse,
};
use tokio::sync::mpsc;

use crate::clipboard::{from_remote, to_remote};

/// What the backend hands to the session loop.
#[derive(Debug)]
pub(crate) enum ClipEvent {
    /// A protocol message the loop must send on the CLIPRDR channel.
    Message(ClipboardMessage),
    /// Text the remote side copied, already converted to local line ends and capped.
    RemoteText(String),
}

/// The text the local side currently offers, shared between the session loop and the backend.
pub(crate) type LocalText = Arc<Mutex<Option<String>>>;

#[derive(Debug)]
pub(crate) struct TextClipboard {
    events: mpsc::UnboundedSender<ClipEvent>,
    local: LocalText,
}

impl TextClipboard {
    pub(crate) fn new(events: mpsc::UnboundedSender<ClipEvent>, local: LocalText) -> Self {
        TextClipboard { events, local }
    }

    fn send(&self, message: ClipboardMessage) {
        // a closed receiver means the session is over; nothing is left to tell
        let _ = self.events.send(ClipEvent::Message(message));
    }

    fn local_formats(&self) -> Vec<ClipboardFormat> {
        let has_text = self.local.lock().map(|t| t.is_some()).unwrap_or(false);
        if has_text {
            vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]
        } else {
            Vec::new()
        }
    }
}

ironrdp_core::impl_as_any!(TextClipboard);

impl CliprdrBackend for TextClipboard {
    fn temporary_directory(&self) -> &str {
        ""
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        // the channel needs an initial format list, empty when there is nothing to offer yet
        self.send(ClipboardMessage::SendInitiateCopy(self.local_formats()));
    }

    fn on_process_negotiated_capabilities(
        &mut self,
        _capabilities: ClipboardGeneralCapabilityFlags,
    ) {
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        if available_formats
            .iter()
            .any(|f| f.id() == ClipboardFormatId::CF_UNICODETEXT)
        {
            self.send(ClipboardMessage::SendInitiatePaste(
                ClipboardFormatId::CF_UNICODETEXT,
            ));
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let text = if request.format == ClipboardFormatId::CF_UNICODETEXT {
            self.local
                .lock()
                .ok()
                .and_then(|t| t.clone())
                .and_then(|t| to_remote(&t))
        } else {
            None
        };
        let response = match text {
            Some(text) => OwnedFormatDataResponse::new_unicode_string(&text),
            None => OwnedFormatDataResponse::new_error(),
        };
        self.send(ClipboardMessage::SendFormatData(response));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            return;
        }
        if let Some(text) = response
            .to_unicode_string()
            .ok()
            .and_then(|t| from_remote(&t))
        {
            let _ = self.events.send(ClipEvent::RemoteText(text));
        }
    }

    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {}

    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _data_id: LockDataId) {}

    fn on_unlock(&mut self, _data_id: LockDataId) {}
}

#[cfg(test)]
mod tests;
