#![forbid(unsafe_code)]

//! Tracks whether the child's byte stream sits at an escape-sequence
//! boundary. Writing our own sequences mid-sequence corrupts both.

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
                        EscapeState::Ground
                    } else {
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

    pub fn is_ground(&self) -> bool {
        matches!(self.escape, EscapeState::Ground) && self.utf8_continuations == 0
    }
}
