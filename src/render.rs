use std::io::{self, Write};

use crate::classify::{select_mode, ExecutionPath, RowDisposition};
use crate::config::Mode;
use crate::layout::{layout_rows, CoordinateMap, LayoutResult};
use crate::terminal::{
    CellWidth, Color, CursorSnapshot, PaneSpan, PhysicalRowSnapshot, ScreenSnapshot, StyleSnapshot,
    UnderlineStyle,
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
        self.repaint_dirty(screen, path, mode, &[])
    }

    pub fn repaint_dirty(
        &mut self,
        screen: &ScreenSnapshot,
        path: &ExecutionPath,
        mode: Mode,
        invalidated_rows: &[u16],
    ) -> io::Result<RepaintSummary> {
        let ComposedLayout { rows, maps } = compose_layout(screen, path, mode);
        let relative_first_repaint = self.previous.is_none() && !invalidated_rows.is_empty();
        let dirty = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let row_index = index as u16;
                (invalidated_rows.contains(&row_index)
                    || (relative_first_repaint && screen.physical_rows.get(index) != Some(row))
                    || (!relative_first_repaint
                        && self
                            .previous
                            .as_ref()
                            .and_then(|previous| previous.get(index))
                            != Some(row)))
                .then_some(row_index)
            })
            .collect::<Vec<_>>();
        let disposition = select_mode(mode, path).disposition;
        let cursor = mapped_cursor(screen, &rows, &maps, disposition);

        let mut relative_row = screen.cursor.row;
        if !dirty.is_empty() {
            self.writer.write_all(b"\x1b[?25l")?;
            for &row_index in &dirty {
                if relative_first_repaint {
                    move_to_relative_row(&mut self.writer, relative_row, row_index)?;
                    self.writer.write_all(b"\r")?;
                    relative_row = row_index;
                } else {
                    write!(self.writer, "\x1b[{};1H", row_index + 1)?;
                }
                write_row(&mut self.writer, &rows[usize::from(row_index)])?;
            }
        }
        if relative_first_repaint {
            move_to_relative_row(&mut self.writer, relative_row, cursor.row)?;
            write!(self.writer, "\x1b[{}G", cursor.col + 1)?;
        } else {
            write!(self.writer, "\x1b[{};{}H", cursor.row + 1, cursor.col + 1)?;
        }
        self.writer.write_all(if cursor.visible {
            b"\x1b[?25h"
        } else {
            b"\x1b[?25l"
        })?;
        self.writer.flush()?;
        let painted = dirty
            .iter()
            .copied()
            .map(usize::from)
            .collect::<std::collections::BTreeSet<_>>();
        self.previous = Some(
            rows.into_iter()
                .enumerate()
                .map(|(index, row)| {
                    if relative_first_repaint && !painted.contains(&index) {
                        screen.physical_rows[index].clone()
                    } else {
                        row
                    }
                })
                .collect(),
        );

        Ok(RepaintSummary {
            rows: dirty,
            cursor,
        })
    }

    pub fn writer_mut(&mut self) -> &mut W {
        &mut self.writer
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}

fn move_to_relative_row(writer: &mut impl Write, from: u16, to: u16) -> io::Result<()> {
    match to.cmp(&from) {
        std::cmp::Ordering::Less => write!(writer, "\x1b[{}A", from - to),
        std::cmp::Ordering::Greater => write!(writer, "\x1b[{}B", to - from),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

struct ComposedLayout {
    rows: Vec<PhysicalRowSnapshot>,
    maps: Vec<Vec<Option<CoordinateMap>>>,
}

fn compose_layout(screen: &ScreenSnapshot, path: &ExecutionPath, mode: Mode) -> ComposedLayout {
    let mut rows = screen.physical_rows.clone();
    let mut maps = vec![vec![None; screen.pane_spans.len()]; rows.len()];
    for (pane_index, &pane) in screen.pane_spans.iter().enumerate() {
        let results = layout_rows(&screen.physical_rows, pane, path, mode);
        for (row_index, result) in results.into_iter().enumerate() {
            replace_pane(&mut rows[row_index], &result, pane);
            maps[row_index][pane_index] = result.coordinates;
        }
    }
    ComposedLayout { rows, maps }
}

fn replace_pane(row: &mut PhysicalRowSnapshot, result: &LayoutResult, pane: PaneSpan) {
    let start = usize::from(pane.start_col).min(row.cells.len());
    let end = usize::from(pane.end_col).min(row.cells.len());
    row.cells[start..end].clone_from_slice(&result.cells[start..end]);
}

fn mapped_cursor(
    screen: &ScreenSnapshot,
    rows: &[PhysicalRowSnapshot],
    maps: &[Vec<Option<CoordinateMap>>],
    disposition: RowDisposition,
) -> CursorSnapshot {
    let original = screen.cursor;
    if disposition == RowDisposition::PassThrough {
        return original;
    }
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
    if disposition == RowDisposition::RecoverVisual {
        return recovered_visual_cursor(
            original,
            &screen.physical_rows[row],
            &rows[row],
            maps[row][pane_index].as_ref(),
            *pane,
        );
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

fn recovered_visual_cursor(
    original: CursorSnapshot,
    painted: &PhysicalRowSnapshot,
    laid_out: &PhysicalRowSnapshot,
    map: Option<&CoordinateMap>,
    pane: PaneSpan,
) -> CursorSnapshot {
    let Some(map) = map else {
        return original;
    };
    let Some(painted_end) = last_glyph_col(painted, pane) else {
        return original;
    };
    if original.col != painted_end {
        return original;
    }
    if last_glyph_col(laid_out, pane).is_none() {
        return original;
    }
    let Some(col) = map.visual_col(map.caret_end) else {
        return original;
    };
    CursorSnapshot { col, ..original }
}

fn last_glyph_col(row: &PhysicalRowSnapshot, pane: PaneSpan) -> Option<u16> {
    glyph_cols(row, pane).next_back()
}

fn glyph_cols(
    row: &PhysicalRowSnapshot,
    pane: PaneSpan,
) -> impl DoubleEndedIterator<Item = u16> + '_ {
    let start = usize::from(pane.start_col).min(row.cells.len());
    let end = usize::from(pane.end_col).min(row.cells.len());
    row.cells[start..end]
        .iter()
        .enumerate()
        .filter(|(_, cell)| !cell.text.trim().is_empty())
        .map(move |(index, _)| (start + index) as u16)
}

fn write_row(writer: &mut impl Write, row: &PhysicalRowSnapshot) -> io::Result<()> {
    let mut style = None;
    let mut hyperlink = None;
    for cell in &row.cells {
        if cell.width == CellWidth::Continuation {
            continue;
        }
        let cell_hyperlink = cell.hyperlink.as_deref();
        if hyperlink != cell_hyperlink {
            writer.write_all(b"\x1b]8;;\x1b\\")?;
            if let Some(uri) = cell_hyperlink {
                writer.write_all(b"\x1b]8;;")?;
                writer.write_all(uri.as_bytes())?;
                writer.write_all(b"\x1b\\")?;
            }
            hyperlink = cell_hyperlink;
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
    if hyperlink.is_some() {
        writer.write_all(b"\x1b]8;;\x1b\\")?;
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
    match style.underline_style {
        UnderlineStyle::None => {
            if style.underline {
                writer.write_all(b";4")?;
            }
        }
        UnderlineStyle::Single => writer.write_all(b";4")?,
        UnderlineStyle::Double => writer.write_all(b";4:2")?,
        UnderlineStyle::Curl => writer.write_all(b";4:3")?,
        UnderlineStyle::Dotted => writer.write_all(b";4:4")?,
        UnderlineStyle::Dashed => writer.write_all(b";4:5")?,
    }
    if let Some(color) = style.underline_color {
        write_color_code(writer, color, 58)?;
    }
    if style.inverse {
        writer.write_all(b";7")?;
    }
    if style.hidden {
        writer.write_all(b";8")?;
    }
    if style.strikeout {
        writer.write_all(b";9")?;
    }
    write_color(writer, style.foreground, true)?;
    write_color(writer, style.background, false)?;
    writer.write_all(b"m")
}

fn write_color(writer: &mut impl Write, color: Color, foreground: bool) -> io::Result<()> {
    write_color_code(writer, color, if foreground { 38 } else { 48 })
}

fn write_color_code(writer: &mut impl Write, color: Color, code: u8) -> io::Result<()> {
    match color {
        Color::Default if code == 38 => writer.write_all(b";39"),
        Color::Default if code == 48 => writer.write_all(b";49"),
        Color::Default => writer.write_all(b";59"),
        Color::Indexed(index) => write!(writer, ";{code};5;{index}"),
        Color::Rgb(red, green, blue) => write!(writer, ";{code};2;{red};{green};{blue}"),
    }
}
