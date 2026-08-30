#![forbid(unsafe_code)]

use std::io::{self, Write};

use serde::Serialize;

use crate::classify::{ExecutionPath, Host, RowDisposition, Selection};
use crate::config::Mode;

#[derive(Serialize)]
pub struct DiagnosticRecord<'a> {
    pub command: &'a str,
    pub host: Option<Host>,
    pub confidence: crate::classify::Confidence,
    pub evidence: &'a [String],
    pub selected_mode: Mode,
    pub row_disposition: RowDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safe_mode_reason: Option<&'a str>,
}

impl<'a> DiagnosticRecord<'a> {
    pub fn new(
        command: &'a str,
        host: Option<Host>,
        mode: Mode,
        path: &'a ExecutionPath,
        selection: &'a Selection,
    ) -> Self {
        Self {
            command,
            host,
            confidence: path.confidence,
            evidence: &path.evidence,
            selected_mode: mode,
            row_disposition: selection.disposition,
            safe_mode_reason: selection.safe_mode_reason.as_deref(),
        }
    }
}

pub struct Diagnostics<W> {
    writer: W,
}

impl<W: Write> Diagnostics<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    pub fn emit(&mut self, record: &DiagnosticRecord<'_>) -> io::Result<()> {
        serde_json::to_writer(&mut self.writer, record)?;
        self.writer.write_all(b"\n")
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}
