#![forbid(unsafe_code)]

//! The transforming half of the proxy: forward the child's bytes untouched,
//! then repair the rows it just painted. Kept out of the platform module so an
//! offline replay of a recording drives this code and not a copy of it.

use std::io::{self, Write};

use crate::classify::{select_mode, ExecutionPath, RowDisposition};
use crate::config::Mode;
use crate::layout::is_rtl_char;
use crate::render::Renderer;
use crate::stream::StreamBoundary;
use crate::terminal::{ScreenSnapshot, TerminalError, TerminalModel};

/// Lets a recording writer see the child's bytes and the resizes, which are
/// otherwise not visible in the stream the terminal receives.
pub trait RelayWriter: Write {
    fn record_input(&mut self, _bytes: &[u8]) {}
    fn record_resize(&mut self, _rows: u16, _cols: u16) {}
}

impl RelayWriter for Vec<u8> {}

pub struct Transform<W: RelayWriter> {
    model: Box<TerminalModel>,
    renderer: Renderer<W>,
    path: ExecutionPath,
    mode: Mode,
    corrected: bool,
    boundary: StreamBoundary,
    pending_rows: Vec<u16>,
    pending_cursor: bool,
    touched: Vec<bool>,
}

impl<W: RelayWriter> Transform<W> {
    pub fn new(
        writer: W,
        rows: u16,
        cols: u16,
        path: ExecutionPath,
        mode: Mode,
    ) -> Result<Self, TerminalError> {
        let mut model = TerminalModel::new(rows, cols)?;
        model.take_dirty_rows();
        Ok(Self {
            model: Box::new(model),
            renderer: Renderer::new(writer),
            path,
            mode,
            corrected: false,
            boundary: StreamBoundary::default(),
            pending_rows: Vec::new(),
            pending_cursor: false,
            touched: vec![false; usize::from(rows)],
        })
    }

    pub fn writer_mut(&mut self) -> &mut W {
        self.renderer.writer_mut()
    }

    pub fn model(&self) -> &TerminalModel {
        &self.model
    }

    pub fn path(&self) -> &ExecutionPath {
        &self.path
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    fn passes_through(&self) -> bool {
        select_mode(self.mode, &self.path).disposition == RowDisposition::PassThrough
    }

    /// Forget which rows the current program painted. Called when another
    /// program takes the pty, before its first byte, so a verdict that lands
    /// later has nothing to repair until that program paints RTL itself. Once
    /// it does, every RTL row on screen is held to the path, the way a direct
    /// launch holds them.
    pub fn mark_generation(&mut self) {
        self.touched.iter_mut().for_each(|row| *row = false);
    }

    /// Hold the rows against another path. Rows the current program painted
    /// that carry RTL are repaired under it right away when the stream is at
    /// ground, and queued for the next feed otherwise.
    pub fn set_path(&mut self, path: ExecutionPath, mode: Mode) -> io::Result<()> {
        let same_verdict = self.mode == mode
            && self.path.order == path.order
            && self.path.wrapping == path.wrapping
            && self.path.confidence == path.confidence;
        self.path = path;
        self.mode = mode;
        if same_verdict || self.passes_through() {
            return Ok(());
        }
        let snapshot = self.model.snapshot();
        let rows = rtl_rows(&snapshot)
            .filter(|row| self.touched.get(usize::from(*row)).copied().unwrap_or(false))
            .collect::<Vec<_>>();
        if rows.is_empty() {
            return Ok(());
        }
        if !self.boundary.is_ground() {
            self.pending_rows.extend(rows);
            self.pending_rows.sort_unstable();
            self.pending_rows.dedup();
            return Ok(());
        }
        self.renderer
            .repaint_dirty(&snapshot, &self.path, self.mode, &rows)?;
        self.corrected = true;
        self.renderer.writer_mut().flush()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> io::Result<()> {
        let before = self.model.cursor();
        let restored = self.corrected && self.boundary.is_ground();
        if restored {
            write!(
                self.renderer.writer_mut(),
                "\x1b[{};{}H",
                before.row + 1,
                before.col + 1
            )?;
        }
        self.renderer.writer_mut().record_input(bytes);
        self.renderer.writer_mut().write_all(bytes)?;
        self.boundary.feed(bytes);
        self.model.feed(bytes);
        let dirty = self
            .model
            .take_dirty_rows()
            .into_iter()
            .map(|row| row.row_index)
            .collect::<Vec<_>>();
        for row in &dirty {
            if let Some(touched) = self.touched.get_mut(usize::from(*row)) {
                *touched = true;
            }
        }
        if self.passes_through() {
            self.pending_rows.clear();
            self.pending_cursor = false;
            if restored {
                self.corrected = false;
            }
            return self.renderer.writer_mut().flush();
        }
        self.pending_rows.extend(dirty);
        let snapshot = self.model.snapshot();
        self.pending_cursor |= before != snapshot.cursor;
        if self.corrected {
            let rows = rtl_rows(&snapshot).collect::<Vec<_>>();
            self.pending_rows.extend(rows);
        }
        self.pending_rows.sort_unstable();
        self.pending_rows.dedup();
        if !self.boundary.is_ground() {
            return self.renderer.writer_mut().flush();
        }
        if !screen_has_rtl(&snapshot) {
            if self.corrected && (!self.pending_rows.is_empty() || self.pending_cursor) {
                self.renderer.repaint_dirty(
                    &snapshot,
                    &self.path,
                    self.mode,
                    &self.pending_rows,
                )?;
            }
            self.corrected = false;
            self.pending_rows.clear();
            self.pending_cursor = false;
            return self.renderer.writer_mut().flush();
        }
        if self.pending_rows.is_empty() && !self.pending_cursor {
            return self.renderer.writer_mut().flush();
        }
        self.renderer
            .repaint_dirty(&snapshot, &self.path, self.mode, &self.pending_rows)?;
        self.corrected = true;
        self.pending_rows.clear();
        self.pending_cursor = false;
        Ok(())
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        self.renderer.writer_mut().record_resize(rows, cols);
        if self.model.resize(rows, cols).is_err() {
            return Ok(());
        }
        let invalidated = self
            .model
            .take_dirty_rows()
            .into_iter()
            .map(|row| row.row_index)
            .collect::<Vec<_>>();
        self.touched = vec![false; usize::from(rows)];
        for row in &invalidated {
            if let Some(touched) = self.touched.get_mut(usize::from(*row)) {
                *touched = true;
            }
        }
        self.pending_rows.clear();
        self.pending_cursor = false;
        if self.passes_through() {
            self.corrected = false;
            return Ok(());
        }
        let snapshot = self.model.snapshot();
        if screen_has_rtl(&snapshot) {
            self.renderer
                .repaint_dirty(&snapshot, &self.path, self.mode, &invalidated)?;
            self.corrected = true;
        } else {
            self.corrected = false;
        }
        Ok(())
    }
}

pub fn rtl_rows(screen: &ScreenSnapshot) -> impl Iterator<Item = u16> + '_ {
    screen
        .physical_rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row.cells
                .iter()
                .any(|cell| cell.text.chars().any(is_rtl_char))
        })
        .map(|(index, _)| index as u16)
}

pub fn screen_has_rtl(screen: &ScreenSnapshot) -> bool {
    rtl_rows(screen).next().is_some()
}
