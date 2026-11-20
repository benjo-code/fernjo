//! Keyboard and mouse events the UI sends to a session.

use serde::Deserialize;

/// Most events one call may carry; a longer batch is refused by the caller.
pub const MAX_INPUT_EVENTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

/// One input event. Coordinates are desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InputEvent {
    Key {
        scancode: u8,
        extended: bool,
        pressed: bool,
    },
    MouseMove {
        x: u16,
        y: u16,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    Wheel {
        vertical: bool,
        delta: i16,
    },
}

#[cfg(test)]
mod tests;
