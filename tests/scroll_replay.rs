use hebrew_tty::classify::{Confidence, ExecutionPath, Order, Wrapping};
use hebrew_tty::config::Mode;
use hebrew_tty::layout::layout_rows;
use hebrew_tty::relay::Transform;
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

/// Drives the real relay and returns (what the terminal was painted, what the
/// proxy meant to paint).
fn replay(rows: u16, cols: u16, chunks: &[Vec<u8>]) -> (Vec<String>, Vec<String>) {
    let path = verified_path();
    let mut transform = Transform::new(Vec::new(), rows, cols, path.clone(), Mode::Auto).unwrap();
    let mut outer = TerminalModel::new(rows, cols).unwrap();

    for chunk in chunks {
        transform.feed(chunk).unwrap();
        let painted = std::mem::take(transform.writer_mut());
        outer.feed(&painted);
    }

    let snapshot = transform.model().snapshot();
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

/// The agent paints a synchronized frame differentially, so anything of ours
/// applied with that frame stays in the cells the frame does not rewrite.
#[test]
fn nothing_of_ours_is_written_inside_a_synchronized_update() {
    let mut transform = Transform::new(Vec::new(), 10, 40, verified_path(), Mode::Auto).unwrap();
    let mut boundary = StreamBoundary::default();

    // One frame, delivered as four pty reads: the middle two land inside it.
    let chunks: Vec<Vec<u8>> = vec![
        b"\x1b[?2026h\x1b[1;1H".to_vec(),
        "שלום עולם".as_bytes().to_vec(),
        "\r\n\x1b[2;1Hעוד שורה".as_bytes().to_vec(),
        b"\x1b[?2026l".to_vec(),
    ];

    let mut painted_after_the_frame = Vec::new();
    for chunk in &chunks {
        transform.feed(chunk).unwrap();
        boundary.feed(chunk);
        let painted = std::mem::take(transform.writer_mut());
        if boundary.is_synchronized() {
            assert_eq!(
                painted, *chunk,
                "our own bytes landed inside the synchronized frame"
            );
        } else {
            painted_after_the_frame = painted;
        }
    }

    // The frame closed, so the repair is owed and must have been paid.
    assert!(
        painted_after_the_frame.len() > chunks[3].len(),
        "the rows were never repaired after the frame closed"
    );
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
