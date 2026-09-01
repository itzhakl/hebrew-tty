use std::collections::BTreeSet;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::{Dimensions, GridCell};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{self, NamedColor};

const DIVIDERS: &[char] = &['│', '┃', '┆', '┇', '┊', '┋', '║'];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn columns(&self) -> usize {
        usize::from(self.cols)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorSnapshot {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Curl,
    Dotted,
    Dashed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StyleSnapshot {
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub underline_style: UnderlineStyle,
    pub underline_color: Option<Color>,
    pub inverse: bool,
    pub hidden: bool,
    pub strikeout: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellWidth {
    Empty,
    Single,
    Wide,
    Continuation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellSnapshot {
    pub text: String,
    pub style: StyleSnapshot,
    pub hyperlink: Option<String>,
    pub width: CellWidth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalRowSnapshot {
    pub cells: Vec<CellSnapshot>,
    pub soft_wrapped: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirtyRowSnapshot {
    pub row_index: u16,
    pub row: PhysicalRowSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneSpan {
    pub start_col: u16,
    pub end_col: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenSnapshot {
    pub size: Size,
    pub physical_rows: Vec<PhysicalRowSnapshot>,
    pub pane_spans: Vec<PaneSpan>,
    pub cursor: CursorSnapshot,
    pub alternate_screen: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalError {
    ZeroSize,
}

impl std::fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroSize => formatter.write_str("terminal dimensions must be non-zero"),
        }
    }
}

impl std::error::Error for TerminalError {}

pub struct TerminalModel {
    term: Term<VoidListener>,
    processor: ansi::Processor,
    size: Size,
    dirty_rows: BTreeSet<u16>,
}

impl TerminalModel {
    pub fn new(rows: u16, cols: u16) -> Result<Self, TerminalError> {
        let size = checked_size(rows, cols)?;
        Ok(Self {
            term: Term::new(Config::default(), &size, VoidListener),
            processor: ansi::Processor::new(),
            size,
            dirty_rows: (0..rows).collect(),
        })
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let before = self.snapshot();
        self.processor.advance(&mut self.term, bytes);
        self.accumulate_dirty(&before);
    }

    /// Offline replay only: one row without the cost of a whole screen.
    pub fn row_text(&self, row: u16) -> String {
        if row >= self.size.rows {
            return String::new();
        }
        let line = Line(i32::from(row));
        (0..self.size.cols)
            .map(|col| {
                let cell = &self.term.grid()[line][Column(usize::from(col))];
                if cell.c == '\0' {
                    ' '
                } else {
                    cell.c
                }
            })
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// Offline replay only: advances the screen without the per-feed snapshot
    /// the dirty accounting costs, which a million recorded records cannot pay.
    pub fn feed_untracked(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), TerminalError> {
        let size = checked_size(rows, cols)?;
        self.term.resize(size);
        self.size = size;
        self.dirty_rows = (0..rows).collect();
        Ok(())
    }

    pub fn snapshot(&self) -> ScreenSnapshot {
        let physical_rows = physical_rows(&self.term, self.size);
        ScreenSnapshot {
            size: self.size,
            pane_spans: pane_spans(&physical_rows, self.size.cols),
            physical_rows,
            cursor: self.cursor(),
            alternate_screen: self.term.mode().contains(TermMode::ALT_SCREEN),
        }
    }

    pub fn cursor(&self) -> CursorSnapshot {
        let point = self.term.grid().cursor.point;
        CursorSnapshot {
            row: point.line.0.max(0) as u16,
            col: point.column.0 as u16,
            visible: self.term.mode().contains(TermMode::SHOW_CURSOR),
        }
    }

    pub fn take_dirty_rows(&mut self) -> Vec<DirtyRowSnapshot> {
        let current = physical_rows(&self.term, self.size);
        std::mem::take(&mut self.dirty_rows)
            .into_iter()
            .filter_map(|row_index| {
                current
                    .get(usize::from(row_index))
                    .cloned()
                    .map(|row| DirtyRowSnapshot { row_index, row })
            })
            .collect()
    }

    fn accumulate_dirty(&mut self, before: &ScreenSnapshot) {
        let after = self.snapshot();
        if before.pane_spans != after.pane_spans {
            self.dirty_rows.extend(0..after.size.rows);
            return;
        }
        for (row_index, row) in after.physical_rows.iter().enumerate() {
            if before.physical_rows.get(row_index) != Some(row) {
                self.dirty_rows.insert(row_index as u16);
            }
        }
    }
}

fn checked_size(rows: u16, cols: u16) -> Result<Size, TerminalError> {
    if rows == 0 || cols == 0 {
        Err(TerminalError::ZeroSize)
    } else {
        Ok(Size { rows, cols })
    }
}

fn physical_rows(term: &Term<VoidListener>, size: Size) -> Vec<PhysicalRowSnapshot> {
    (0..size.rows)
        .map(|row| {
            let line = Line(i32::from(row));
            let mut cells = (0..size.cols)
                .map(|col| cell_snapshot(&term.grid()[line][Column(usize::from(col))]))
                .collect::<Vec<_>>();
            sanitize_wide_cells(&mut cells);
            PhysicalRowSnapshot {
                cells,
                soft_wrapped: term.grid()[line][Column(usize::from(size.cols - 1))]
                    .flags
                    .contains(Flags::WRAPLINE),
            }
        })
        .collect()
}

fn sanitize_wide_cells(cells: &mut [CellSnapshot]) {
    for col in 0..cells.len() {
        let complete_wide = cells[col].width == CellWidth::Wide
            && cells
                .get(col + 1)
                .is_some_and(|cell| cell.width == CellWidth::Continuation);
        let valid_continuation = cells[col].width == CellWidth::Continuation
            && col > 0
            && cells[col - 1].width == CellWidth::Wide;
        if (cells[col].width == CellWidth::Wide && !complete_wide)
            || (cells[col].width == CellWidth::Continuation && !valid_continuation)
        {
            cells[col].text.clear();
            cells[col].width = CellWidth::Empty;
        }
    }
}

fn cell_snapshot(cell: &Cell) -> CellSnapshot {
    let width = if cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
        CellWidth::Empty
    } else if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
        CellWidth::Continuation
    } else if cell.flags.contains(Flags::WIDE_CHAR) {
        CellWidth::Wide
    } else if cell.is_empty() {
        CellWidth::Empty
    } else {
        CellWidth::Single
    };
    let mut text = match width {
        CellWidth::Empty | CellWidth::Continuation => String::new(),
        CellWidth::Single | CellWidth::Wide => cell.c.to_string(),
    };
    if let Some(zerowidth) = cell.zerowidth() {
        text.extend(zerowidth);
    }
    CellSnapshot {
        text,
        style: StyleSnapshot {
            foreground: color(cell.fg, NamedColor::Foreground),
            background: color(cell.bg, NamedColor::Background),
            bold: cell.flags.contains(Flags::BOLD),
            dim: cell.flags.contains(Flags::DIM),
            italic: cell.flags.contains(Flags::ITALIC),
            underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
            underline_style: underline_style(cell.flags),
            underline_color: cell
                .underline_color()
                .map(|value| color(value, NamedColor::Foreground)),
            inverse: cell.flags.contains(Flags::INVERSE),
            hidden: cell.flags.contains(Flags::HIDDEN),
            strikeout: cell.flags.contains(Flags::STRIKEOUT),
        },
        hyperlink: cell.hyperlink().map(|value| value.uri().to_owned()),
        width,
    }
}

fn underline_style(flags: Flags) -> UnderlineStyle {
    if flags.contains(Flags::DOUBLE_UNDERLINE) {
        UnderlineStyle::Double
    } else if flags.contains(Flags::UNDERCURL) {
        UnderlineStyle::Curl
    } else if flags.contains(Flags::DOTTED_UNDERLINE) {
        UnderlineStyle::Dotted
    } else if flags.contains(Flags::DASHED_UNDERLINE) {
        UnderlineStyle::Dashed
    } else if flags.contains(Flags::UNDERLINE) {
        UnderlineStyle::Single
    } else {
        UnderlineStyle::None
    }
}

fn color(color: ansi::Color, default: NamedColor) -> Color {
    match color {
        ansi::Color::Named(named) if named == default => Color::Default,
        ansi::Color::Named(named) if (named as usize) < 16 => Color::Indexed(named as u8),
        ansi::Color::Named(_) => Color::Default,
        ansi::Color::Indexed(index) => Color::Indexed(index),
        ansi::Color::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

fn pane_spans(rows: &[PhysicalRowSnapshot], cols: u16) -> Vec<PaneSpan> {
    if rows.len() < 8 {
        return vec![PaneSpan {
            start_col: 0,
            end_col: cols,
        }];
    }
    let threshold = rows.len() * 9;
    let dividers = (0..usize::from(cols))
        .filter(|&col| {
            rows.iter()
                .filter(|row| {
                    let mut chars = row.cells[col].text.chars();
                    chars.next().is_some_and(|ch| DIVIDERS.contains(&ch)) && chars.next().is_none()
                })
                .count()
                * 10
                >= threshold
        })
        .collect::<BTreeSet<_>>();
    if dividers.is_empty() {
        return vec![PaneSpan {
            start_col: 0,
            end_col: cols,
        }];
    }
    let mut spans = Vec::new();
    let mut start = 0;
    for divider in dividers {
        if start < divider {
            spans.push(PaneSpan {
                start_col: start as u16,
                end_col: divider as u16,
            });
        }
        start = divider + 1;
    }
    if start < usize::from(cols) {
        spans.push(PaneSpan {
            start_col: start as u16,
            end_col: cols,
        });
    }
    spans
}
