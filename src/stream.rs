#![forbid(unsafe_code)]

//! Tracks whether the child's byte stream sits at an escape-sequence
//! boundary. Writing our own sequences mid-sequence corrupts both.
//!
//! A synchronized update (`CSI ? 2026 h` ... `l`) is a boundary of its own.
//! The terminal holds the whole frame back and applies it at once, and the
//! agent paints such a frame differentially - it rewrites only the cells it
//! believes changed. Anything we inject lands inside that frame and is applied
//! with it, so the cells the frame does not rewrite keep what we put there,
//! and neither side ever repaints them again.

#[derive(Clone, Copy, Default)]
pub enum EscapeState {
    #[default]
    Ground,
    Escape,
    Csi,
    String,
    StringEscape,
}

#[derive(Default)]
pub struct StreamBoundary {
    escape: EscapeState,
    utf8_continuations: u8,
    parameters: Vec<u8>,
    synchronized: bool,
}

impl StreamBoundary {
    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.utf8_continuations > 0 {
                if byte & 0b1100_0000 == 0b1000_0000 {
                    self.utf8_continuations -= 1;
                    continue;
                }
                self.utf8_continuations = 0;
            }
            if matches!(byte, 0x18 | 0x1a) {
                self.escape = EscapeState::Ground;
                continue;
            }
            if byte == 0x1b && !matches!(self.escape, EscapeState::String) {
                self.parameters.clear();
                self.escape = EscapeState::Escape;
                continue;
            }
            self.escape = match self.escape {
                EscapeState::Ground => match byte {
                    0x90 | 0x9d..=0x9f => EscapeState::String,
                    0x9b => EscapeState::Csi,
                    0xc2..=0xdf => {
                        self.utf8_continuations = 1;
                        EscapeState::Ground
                    }
                    0xe0..=0xef => {
                        self.utf8_continuations = 2;
                        EscapeState::Ground
                    }
                    0xf0..=0xf4 => {
                        self.utf8_continuations = 3;
                        EscapeState::Ground
                    }
                    _ => EscapeState::Ground,
                },
                EscapeState::Escape => match byte {
                    b'[' => EscapeState::Csi,
                    b']' | b'P' | b'_' | b'^' => EscapeState::String,
                    0x20..=0x2f => EscapeState::Escape,
                    _ => EscapeState::Ground,
                },
                EscapeState::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        if self.parameters == b"?2026" {
                            match byte {
                                b'h' => self.synchronized = true,
                                b'l' => self.synchronized = false,
                                _ => {}
                            }
                        }
                        self.parameters.clear();
                        EscapeState::Ground
                    } else {
                        if self.parameters.len() < 16 {
                            self.parameters.push(byte);
                        }
                        EscapeState::Csi
                    }
                }
                EscapeState::String => match byte {
                    0x07 | 0x9c => EscapeState::Ground,
                    0x1b => EscapeState::StringEscape,
                    _ => EscapeState::String,
                },
                EscapeState::StringEscape => {
                    if byte == b'\\' {
                        EscapeState::Ground
                    } else {
                        EscapeState::String
                    }
                }
            };
        }
    }

    /// True only where our own bytes can be written without being swallowed by
    /// somebody else's sequence or somebody else's frame.
    pub fn is_ground(&self) -> bool {
        matches!(self.escape, EscapeState::Ground)
            && self.utf8_continuations == 0
            && !self.synchronized
    }

    /// Inside a synchronized update the child is still mid-frame.
    pub fn is_synchronized(&self) -> bool {
        self.synchronized
    }
}
