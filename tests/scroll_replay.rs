use std::io::Write;

use hebrew_tty::classify::{Confidence, ExecutionPath, Order, Wrapping};
use hebrew_tty::config::Mode;
use hebrew_tty::layout::layout_rows;
use hebrew_tty::render::Renderer;
use hebrew_tty::stream::StreamBoundary;
use hebrew_tty::terminal::{PaneSpan, PhysicalRowSnapshot, TerminalModel};

fn verified_path() -> ExecutionPath {
    ExecutionPath {
        order: Some(Order::Visual),
        wrapping: Some(Wrapping::PostBidi),
        confidence: Confidence::Verified,
        evidence: Vec::new(),
    }
}

fn pane(cols: u16) -> PaneSpan {
    PaneSpan {
        start_col: 0,
        end_col: cols,
    }
}

fn text(row: &PhysicalRowSnapshot) -> String {
    row.cells
        .iter()
        .map(|cell| {
            if cell.text.is_empty() {
                " "
            } else {
                cell.text.as_str()
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn rtl_rows(screen: &hebrew_tty::terminal::ScreenSnapshot) -> impl Iterator<Item = u16> + '_ {
    screen
        .physical_rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row.cells
                .iter()
                .any(|cell| cell.text.chars().any(hebrew_tty::layout::is_rtl_char))
        })
        .map(|(index, _)| index as u16)
}

fn screen_has_rtl(screen: &hebrew_tty::terminal::ScreenSnapshot) -> bool {
    rtl_rows(screen).next().is_some()
}

/// Drives the same order of writes `OutputRelay::feed` performs, into a second
/// model standing in for the real terminal, and returns (real screen, intended).
fn replay(rows: u16, cols: u16, chunks: &[Vec<u8>]) -> (Vec<String>, Vec<String>) {
    let path = verified_path();
    let mut model = TerminalModel::new(rows, cols).unwrap();
    let mut outer = TerminalModel::new(rows, cols).unwrap();
    let mut renderer = Renderer::new(Vec::new());
    model.take_dirty_rows();
    let mut corrected = false;

    let mut boundary = StreamBoundary::default();
    let mut pending_rows: Vec<u16> = Vec::new();
    let mut pending_cursor = false;

    for chunk in chunks {
        let before = model.cursor();
        if corrected && boundary.is_ground() {
            write!(
                renderer.writer_mut(),
                "\x1b[{};{}H",
                before.row + 1,
                before.col + 1
            )
            .unwrap();
        }
        renderer.writer_mut().write_all(chunk).unwrap();
        boundary.feed(chunk);
        model.feed(chunk);
        pending_rows.extend(model.take_dirty_rows().into_iter().map(|row| row.row_index));
        let snapshot = model.snapshot();
        pending_cursor |= before != snapshot.cursor;
        if corrected {
            pending_rows.extend(rtl_rows(&snapshot));
        }
        pending_rows.sort_unstable();
        pending_rows.dedup();
        if boundary.is_ground() {
            if !screen_has_rtl(&snapshot) {
                if corrected && (!pending_rows.is_empty() || pending_cursor) {
                    renderer
                        .repaint_dirty(&snapshot, &path, Mode::Auto, &pending_rows)
                        .unwrap();
                }
                corrected = false;
                pending_rows.clear();
                pending_cursor = false;
            } else if !pending_rows.is_empty() || pending_cursor {
                renderer
                    .repaint_dirty(&snapshot, &path, Mode::Auto, &pending_rows)
                    .unwrap();
                corrected = true;
                pending_rows.clear();
                pending_cursor = false;
            }
        }
        let bytes = std::mem::take(renderer.writer_mut());
        outer.feed(&bytes);
    }

    let snapshot = model.snapshot();
    let intended = layout_rows(&snapshot.physical_rows, pane(cols), &path, Mode::Auto)
        .into_iter()
        .map(|result| {
            text(&PhysicalRowSnapshot {
                cells: result.cells,
                soft_wrapped: false,
            })
        })
        .collect::<Vec<_>>();
    let real = outer
        .snapshot()
        .physical_rows
        .iter()
        .map(text)
        .collect::<Vec<_>>();
    (real, intended)
}

#[test]
fn scrolling_output_lands_on_the_same_screen_the_model_holds() {
    let chunks = (0..14)
        .map(|index| format!("שורה {index} ok\r\n").into_bytes())
        .collect::<Vec<_>>();
    let (real, intended) = replay(6, 24, &chunks);
    assert_eq!(real, intended);
}

#[test]
fn a_redrawn_bottom_box_survives_the_output_above_it_scrolling() {
    let rows = 10u16;
    let cols = 30u16;
    let mut chunks = Vec::new();
    for index in 0..20 {
        chunks.push(format!("שורה ארוכה של פלט מספר {index}\r\n").into_bytes());
        chunks
            .push(format!("\x1b[{};1H\x1b[2K> קלט {index}\x1b[{};1H", rows, rows - 1).into_bytes());
    }
    let (real, intended) = replay(rows, cols, &chunks);
    assert_eq!(real, intended);

    // A pty read splits wherever it likes, mid escape sequence included.
    for size in [1usize, 3, 7, 64] {
        let joined = chunks.concat();
        let split = joined.chunks(size).map(<[u8]>::to_vec).collect::<Vec<_>>();
        let (real, intended) = replay(rows, cols, &split);
        assert_eq!(real, intended, "split every {size} bytes");
    }
}

#[test]
fn a_row_filled_to_the_last_column_still_wraps_on_the_real_screen() {
    let mut chunks = (0..4)
        .map(|index| format!("שלום עול{index}ab").into_bytes())
        .collect::<Vec<_>>();
    chunks.push(b"cd ef".to_vec());
    let (real, intended) = replay(4, 12, &chunks);
    assert_eq!(real, intended);
}
