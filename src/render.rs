use std::io::{self, Write};

use crate::classify::ExecutionPath;
use crate::config::Mode;
use crate::layout::{layout_rows, CoordinateMap, LayoutResult};
use crate::terminal::{
    CellWidth, Color, CursorSnapshot, PaneSpan, PhysicalRowSnapshot, ScreenSnapshot, StyleSnapshot,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepaintSummary {
    pub rows: Vec<u16>,
    pub cursor: CursorSnapshot,
}

pub struct Renderer<W> {
    writer: W,
    previous: Option<Vec<PhysicalRowSnapshot>>,
}

impl<W: Write> Renderer<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            previous: None,
        }
    }

    pub fn repaint(
        &mut self,
        screen: &ScreenSnapshot,
        path: &ExecutionPath,
        mode: Mode,
    ) -> io::Result<RepaintSummary> {
        let (rows, maps) = compose_layout(screen, path, mode);
        let dirty = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                (self
                    .previous
                    .as_ref()
                    .and_then(|previous| previous.get(index))
                    != Some(row))
                .then_some(index as u16)
            })
            .collect::<Vec<_>>();
        let cursor = mapped_cursor(screen, &maps);

        if !dirty.is_empty() {
            self.writer.write_all(b"\x1b[?25l")?;
            for &row_index in &dirty {
                write!(self.writer, "\x1b[{};1H", row_index + 1)?;
                write_row(&mut self.writer, &rows[usize::from(row_index)])?;
            }
        }
        write!(self.writer, "\x1b[{};{}H", cursor.row + 1, cursor.col + 1)?;
        self.writer.write_all(if cursor.visible {
            b"\x1b[?25h"
        } else {
            b"\x1b[?25l"
        })?;
        self.writer.flush()?;
        self.previous = Some(rows);

        Ok(RepaintSummary {
            rows: dirty,
            cursor,
        })
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}

fn compose_layout(
    screen: &ScreenSnapshot,
    path: &ExecutionPath,
    mode: Mode,
) -> (Vec<PhysicalRowSnapshot>, Vec<Vec<Option<CoordinateMap>>>) {
    let mut rows = screen.physical_rows.clone();
    let mut maps = vec![vec![None; screen.pane_spans.len()]; rows.len()];
    for (pane_index, &pane) in screen.pane_spans.iter().enumerate() {
        let results = layout_rows(&screen.physical_rows, pane, path, mode);
        for (row_index, result) in results.into_iter().enumerate() {
            replace_pane(&mut rows[row_index], &result, pane);
            maps[row_index][pane_index] = result.coordinates;
        }
    }
    (rows, maps)
}

fn replace_pane(row: &mut PhysicalRowSnapshot, result: &LayoutResult, pane: PaneSpan) {
    let start = usize::from(pane.start_col).min(row.cells.len());
    let end = usize::from(pane.end_col).min(row.cells.len());
    row.cells[start..end].clone_from_slice(&result.cells[start..end]);
}

fn mapped_cursor(screen: &ScreenSnapshot, maps: &[Vec<Option<CoordinateMap>>]) -> CursorSnapshot {
    let original = screen.cursor;
    let Some((pane_index, pane)) = screen
        .pane_spans
        .iter()
        .enumerate()
        .find(|(_, pane)| (pane.start_col..pane.end_col).contains(&original.col))
    else {
        return original;
    };
    let row = usize::from(original.row);
    if row >= screen.physical_rows.len() {
        return original;
    }
    let mut group_start = row;
    while group_start > 0 && screen.physical_rows[group_start - 1].soft_wrapped {
        group_start -= 1;
    }
    let mut group_end = row;
    while group_end + 1 < screen.physical_rows.len() && screen.physical_rows[group_end].soft_wrapped
    {
        group_end += 1;
    }
    let pane_width = usize::from(pane.end_col - pane.start_col);
    let logical_col =
        (row - group_start) * pane_width + usize::from(original.col.saturating_sub(pane.start_col));
    for (output_row, row_maps) in maps
        .iter()
        .enumerate()
        .take(group_end + 1)
        .skip(group_start)
    {
        if let Some(visual_col) = row_maps[pane_index]
            .as_ref()
            .and_then(|map| map.visual_col(logical_col))
        {
            return CursorSnapshot {
                row: output_row as u16,
                col: visual_col,
                visible: original.visible,
            };
        }
    }
    original
}

fn write_row(writer: &mut impl Write, row: &PhysicalRowSnapshot) -> io::Result<()> {
    let mut style = None;
    for cell in &row.cells {
        if cell.width == CellWidth::Continuation {
            continue;
        }
        if style != Some(cell.style) {
            write_style(writer, cell.style)?;
            style = Some(cell.style);
        }
        if cell.text.is_empty() {
            writer.write_all(b" ")?;
        } else {
            writer.write_all(cell.text.as_bytes())?;
        }
    }
    writer.write_all(b"\x1b[0m")
}

fn write_style(writer: &mut impl Write, style: StyleSnapshot) -> io::Result<()> {
    writer.write_all(b"\x1b[0")?;
    if style.bold {
        writer.write_all(b";1")?;
    }
    if style.dim {
        writer.write_all(b";2")?;
    }
    if style.italic {
        writer.write_all(b";3")?;
    }
    if style.underline {
        writer.write_all(b";4")?;
    }
    if style.inverse {
        writer.write_all(b";7")?;
    }
    write_color(writer, style.foreground, true)?;
    write_color(writer, style.background, false)?;
    writer.write_all(b"m")
}

fn write_color(writer: &mut impl Write, color: Color, foreground: bool) -> io::Result<()> {
    match color {
        Color::Default => writer.write_all(if foreground { b";39" } else { b";49" }),
        Color::Indexed(index) => write!(writer, ";{};5;{index}", if foreground { 38 } else { 48 }),
        Color::Rgb(red, green, blue) => write!(
            writer,
            ";{};2;{red};{green};{blue}",
            if foreground { 38 } else { 48 }
        ),
    }
}
