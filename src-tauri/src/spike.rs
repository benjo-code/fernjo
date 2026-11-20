//! THROWAWAY (branch `spike/m08-rdp-canvas`, never merged): synthetic RDP-like region messages on a
//! Tauri channel, to measure the webview path (channel cost, copy, `putImageData`) without any RDP
//! host. The UI side is `ui/src/spike/SpikePage.svelte` (open the app with the URL hash `#spike`).
//!
//! Wire format of one message (same as the planned real transport): `u16 x, y, width, height`
//! little endian, then `width * height * 4` bytes of RGBA.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::sync::Semaphore;

static CREDIT: Mutex<Option<Arc<Semaphore>>> = Mutex::new(None);

fn region(mode: &str, frame: u32, width: u16, height: u16) -> (u16, u16, u16, u16) {
    match mode {
        // the whole desktop every time
        "full" => (0, 0, width, height),
        // a large band that moves a little, like a scrolled document
        "scroll" => (0, ((frame * 20) % 100) as u16, width, height.min(900)),
        // a window-sized rectangle that moves around, like a dragged window
        _ => {
            let w = 400.min(width);
            let h = 300.min(height);
            (
                ((frame * 7) % u32::from(width - w + 1)) as u16,
                ((frame * 5) % u32::from(height - h + 1)) as u16,
                w,
                h,
            )
        }
    }
}

pub mod handlers {
    use super::*;

    /// Sends region messages of `mode` for `seconds`, never more than `in_flight` unacknowledged ones
    /// (`spike_ack` returns a credit). Returns how many messages were sent.
    #[tauri::command]
    pub async fn spike_stream(
        on_message: Channel<InvokeResponseBody>,
        width: u16,
        height: u16,
        mode: String,
        seconds: u32,
        in_flight: u32,
    ) -> Result<u32, String> {
        let credit = Arc::new(Semaphore::new(in_flight.max(1) as usize));
        *CREDIT.lock().map_err(|e| e.to_string())? = Some(credit.clone());
        let end = Instant::now() + Duration::from_secs(u64::from(seconds));
        let mut pixels: Vec<u8> = Vec::new();
        let mut frame = 0u32;
        while Instant::now() < end {
            let Ok(Ok(permit)) =
                tokio::time::timeout(Duration::from_secs(2), credit.clone().acquire_owned()).await
            else {
                break;
            };
            permit.forget();
            let (x, y, w, h) = region(&mode, frame, width, height);
            let len = usize::from(w) * usize::from(h) * 4;
            if pixels.len() != len {
                pixels = vec![0x40; len];
            }
            // change something every frame so no layer can cache the message
            let at = (frame as usize * 4099) % len.max(1);
            pixels[at] = pixels[at].wrapping_add(1);
            let mut message = Vec::with_capacity(8 + len);
            for v in [x, y, w, h] {
                message.extend_from_slice(&v.to_le_bytes());
            }
            message.extend_from_slice(&pixels);
            on_message
                .send(InvokeResponseBody::Raw(message))
                .map_err(|e| e.to_string())?;
            frame += 1;
        }
        Ok(frame)
    }

    /// The page painted one message: another one may be sent.
    #[tauri::command]
    pub fn spike_ack() {
        if let Ok(guard) = CREDIT.lock()
            && let Some(credit) = guard.as_ref()
        {
            credit.add_permits(1);
        }
    }
}
