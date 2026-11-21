//! The remote desktop as one shared RGBA buffer.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::RdpError;

/// Largest width or height of a remote desktop the client accepts.
pub const MAX_DESKTOP_DIMENSION: u16 = 8192;

/// A rectangle of the desktop in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    /// The smallest rectangle that contains both.
    pub fn union(self, other: Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (u32::from(self.x) + u32::from(self.width))
            .max(u32::from(other.x) + u32::from(other.width));
        let bottom = (u32::from(self.y) + u32::from(self.height))
            .max(u32::from(other.y) + u32::from(other.height));
        Rect {
            x,
            y,
            width: u16::try_from(right - u32::from(x)).unwrap_or(u16::MAX),
            height: u16::try_from(bottom - u32::from(y)).unwrap_or(u16::MAX),
        }
    }

    /// The part of the rectangle that lies inside a `w` x `h` desktop, or `None` when nothing does.
    pub fn clamp_to(self, w: u16, h: u16) -> Option<Rect> {
        if self.x >= w || self.y >= h || self.width == 0 || self.height == 0 {
            return None;
        }
        Some(Rect {
            x: self.x,
            y: self.y,
            width: self.width.min(w - self.x),
            height: self.height.min(h - self.y),
        })
    }
}

/// The pixels of one session. Cloning is cheap and shares the buffer.
#[derive(Debug, Clone)]
pub struct Framebuffer {
    inner: Arc<Mutex<Pixels>>,
}

#[derive(Debug)]
struct Pixels {
    width: u16,
    height: u16,
    /// `width * height * 4` bytes, RGBA, rows top to bottom.
    data: Vec<u8>,
}

/// Largest desktop in pixels (8192 by 4320, about 141 MB as RGBA): each side may reach
/// [`MAX_DESKTOP_DIMENSION`], but not both at once, so one session cannot ask for gigabytes of
/// framebuffer, decoded image and region copies.
pub const MAX_DESKTOP_PIXELS: u32 = 8192 * 4320;

/// A size is acceptable when neither side is zero or above [`MAX_DESKTOP_DIMENSION`] and the area
/// is at most [`MAX_DESKTOP_PIXELS`]; this is checked before anything is allocated, so a hostile
/// server cannot make the client allocate.
fn check_size(width: u16, height: u16) -> Result<(), RdpError> {
    if width == 0
        || height == 0
        || width > MAX_DESKTOP_DIMENSION
        || height > MAX_DESKTOP_DIMENSION
        || u32::from(width) * u32::from(height) > MAX_DESKTOP_PIXELS
    {
        return Err(RdpError::DesktopTooLarge);
    }
    Ok(())
}

impl Framebuffer {
    /// A black, transparent desktop. Refuses a zero or oversize size with `desktop_too_large`.
    pub fn new(width: u16, height: u16) -> Result<Framebuffer, RdpError> {
        check_size(width, height)?;
        Ok(Framebuffer {
            inner: Arc::new(Mutex::new(Pixels {
                width,
                height,
                data: vec![0; usize::from(width) * usize::from(height) * 4],
            })),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Pixels> {
        // a panic while holding the lock leaves plain bytes behind, which are still usable
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Changes the size and blanks the content (the server repaints everything after a resize).
    pub fn resize(&self, w: u16, h: u16) -> Result<(), RdpError> {
        check_size(w, h)?;
        let mut pixels = self.lock();
        pixels.width = w;
        pixels.height = h;
        pixels.data = vec![0; usize::from(w) * usize::from(h) * 4];
        Ok(())
    }

    pub fn size(&self) -> (u16, u16) {
        let pixels = self.lock();
        (pixels.width, pixels.height)
    }

    /// Writes `rgba` (tightly packed rows of the unclipped `r`) into the desktop. The part of `r`
    /// outside the desktop is ignored, and so is any part that `rgba` is too short for.
    pub fn write_region(&self, r: Rect, rgba: &[u8]) {
        let mut pixels = self.lock();
        let Some(clipped) = r.clamp_to(pixels.width, pixels.height) else {
            return;
        };
        let stride = usize::from(pixels.width) * 4;
        let src_stride = usize::from(r.width) * 4;
        let row_bytes = usize::from(clipped.width) * 4;
        for row in 0..usize::from(clipped.height) {
            let src = row * src_stride;
            let Some(from) = rgba.get(src..src + row_bytes) else {
                break;
            };
            let dst = (usize::from(clipped.y) + row) * stride + usize::from(clipped.x) * 4;
            pixels.data[dst..dst + row_bytes].copy_from_slice(from);
        }
    }

    /// The pixels of `r` clipped to the desktop, tightly packed RGBA (empty when `r` lies
    /// outside). The clipped rectangle is `r.clamp_to(width, height)`.
    pub fn copy_region(&self, r: Rect) -> Vec<u8> {
        let pixels = self.lock();
        let Some(clipped) = r.clamp_to(pixels.width, pixels.height) else {
            return Vec::new();
        };
        let stride = usize::from(pixels.width) * 4;
        let row_bytes = usize::from(clipped.width) * 4;
        let mut out = Vec::with_capacity(row_bytes * usize::from(clipped.height));
        for row in 0..usize::from(clipped.height) {
            let from = (usize::from(clipped.y) + row) * stride + usize::from(clipped.x) * 4;
            out.extend_from_slice(&pixels.data[from..from + row_bytes]);
        }
        out
    }
}

#[cfg(test)]
mod tests;
