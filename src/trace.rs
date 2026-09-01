#![forbid(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io::{self, Write};

pub const TRACE_ENV: &str = "HEBREW_TTY_TRACE";

/// Records both sides of the relay: `<` is what the child wrote, `>` is what
/// the real terminal received. Replaying one into a model and the other into a
/// second model is the only way to see them drift apart.
pub struct TraceWriter<W> {
    inner: W,
    sink: Option<File>,
}

impl<W> TraceWriter<W> {
    pub fn new(inner: W) -> Self {
        let sink = std::env::var_os(TRACE_ENV).and_then(|path| {
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
                .ok()
        });
        Self { inner, sink }
    }

    pub fn is_recording(&self) -> bool {
        self.sink.is_some()
    }

    pub fn record_input(&mut self, bytes: &[u8]) {
        self.record(b'<', bytes);
    }

    pub fn record_resize(&mut self, rows: u16, cols: u16) {
        if let Some(sink) = self.sink.as_mut() {
            let _ = writeln!(sink, "r {rows} {cols}");
        }
    }

    fn record(&mut self, tag: u8, bytes: &[u8]) {
        let Some(sink) = self.sink.as_mut() else {
            return;
        };
        let mut line = String::with_capacity(bytes.len() * 2 + 2);
        line.push(tag as char);
        line.push(' ');
        for byte in bytes {
            line.push_str(&format!("{byte:02x}"));
        }
        let _ = writeln!(sink, "{line}");
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for TraceWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.record(b'>', &buf[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(sink) = self.sink.as_mut() {
            let _ = sink.flush();
        }
        self.inner.flush()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceRecord {
    Input(Vec<u8>),
    Output(Vec<u8>),
    Resize { rows: u16, cols: u16 },
}

pub fn parse_trace(text: &str) -> Vec<TraceRecord> {
    text.lines()
        .filter_map(|line| {
            let (tag, rest) = line.split_once(' ')?;
            match tag {
                "<" => Some(TraceRecord::Input(decode_hex(rest)?)),
                ">" => Some(TraceRecord::Output(decode_hex(rest)?)),
                "r" => {
                    let (rows, cols) = rest.split_once(' ')?;
                    Some(TraceRecord::Resize {
                        rows: rows.parse().ok()?,
                        cols: cols.parse().ok()?,
                    })
                }
                _ => None,
            }
        })
        .collect()
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).ok())
        .collect()
}
