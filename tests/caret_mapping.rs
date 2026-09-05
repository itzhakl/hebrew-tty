use hebrew_tty::classify::{Confidence, ExecutionPath, Order, Wrapping};
use hebrew_tty::config::Mode;
use hebrew_tty::layout::{layout_row, layout_rows};
use hebrew_tty::render::Renderer;
use hebrew_tty::terminal::{PaneSpan, TerminalModel};

#[test]
fn hebrew_caret_boundaries_share_the_row_permutation() {
    let mut model = TerminalModel::new(2, 8).unwrap();
    model.feed("אבגד".as_bytes());
    let snapshot = model.snapshot();
    let result = layout_row(
        &snapshot.physical_rows[0],
        pane(8),
        &verified_path(Order::Logical),
        Mode::Auto,
    );
    let map = result.coordinates.unwrap();

    assert_eq!(map.logical_to_visual, [8, 7, 6, 5, 4]);
    for logical in map.logical_start..=map.logical_end {
        let visual = map.visual_col(logical).unwrap();
        assert_eq!(map.logical_col(visual), Some(logical));
    }
}

#[test]
fn streaming_growth_keeps_the_end_caret_against_the_last_grapheme() {
    for logical in ["א", "אב", "אבג", "אבגד", "אבגדה"] {
        let mut model = TerminalModel::new(2, 12).unwrap();
        model.feed(logical.as_bytes());
        let snapshot = model.snapshot();
        let result = layout_row(
            &snapshot.physical_rows[0],
            pane(12),
            &verified_path(Order::Logical),
            Mode::Auto,
        );
        let map = result.coordinates.unwrap();
        let caret = map.visual_col(logical.chars().count()).unwrap();
        let first_glyph = result
            .cells
            .iter()
            .position(|cell| !cell.text.is_empty())
            .unwrap() as u16;
        assert_eq!(caret, first_glyph, "{logical}");
    }
}

#[test]
fn visual_order_cup_columns_remain_physical_while_typing_rtl() {
    let mut model = TerminalModel::new(2, 65).unwrap();
    model.feed(format!("\x1b[1;1H\x1b[2K{}דגבא\x1b[1;63H", " ".repeat(59)).as_bytes());
    let path = verified_path(Order::Visual);
    let mut renderer = Renderer::new(Vec::new());
    let first_snapshot = model.snapshot();
    assert_eq!(first_snapshot.cursor.col, 62);
    let first = renderer
        .repaint(&first_snapshot, &path, Mode::Auto)
        .unwrap();

    model.feed(format!("\x1b[1;1H\x1b[2K{}הדגבא\x1b[1;62H", " ".repeat(57)).as_bytes());
    let second_snapshot = model.snapshot();
    assert_eq!(second_snapshot.cursor.col, 61);
    let second = renderer
        .repaint(&second_snapshot, &path, Mode::Auto)
        .unwrap();

    assert_eq!((first.cursor.col, second.cursor.col), (61, 60));
}

#[test]
fn recovered_visual_rows_that_were_flushed_right_carry_their_caret() {
    let mut model = TerminalModel::new(30, 100).unwrap();
    model.feed("\x1b[H\r\x1b[3C\x1b[24Bםלוע םולש\x1b[25;12H".as_bytes());
    let snapshot = model.snapshot();
    assert_eq!(snapshot.cursor.col, 11);

    let path = verified_path(Order::Visual);
    let result = layout_row(&snapshot.physical_rows[24], pane(100), &path, Mode::Auto);
    let run_start = result
        .cells
        .iter()
        .position(|cell| !cell.text.trim().is_empty())
        .unwrap() as u16;
    assert_eq!(run_start, 91);

    let mut renderer = Renderer::new(Vec::new());
    let repainted = renderer.repaint(&snapshot, &path, Mode::Auto).unwrap();
    assert_eq!(
        repainted.cursor.col, run_start,
        "the caret belongs on the run's leading cell, against the newest grapheme"
    );
}

#[test]
fn visual_paragraph_continuation_preserves_physical_cursor() {
    let mut model = TerminalModel::new(3, 20).unwrap();
    model.feed("\x1b[1;1Hםולש\x1b[2;1Henglish\x1b[2;8H".as_bytes());
    let snapshot = model.snapshot();
    let mut renderer = Renderer::new(Vec::new());
    let result = renderer
        .repaint(&snapshot, &verified_path(Order::Visual), Mode::Auto)
        .unwrap();

    assert_eq!(result.cursor.row, 1);
    assert_eq!(result.cursor.col, 7);
}

#[test]
fn first_relative_repaint_paints_dependent_composed_rows() {
    let path = verified_path(Order::Logical);
    let mut model = TerminalModel::new(3, 20).unwrap();
    model.feed("\x1b[1;1Hשלום\x1b[2;1Henglish".as_bytes());
    let mut renderer = Renderer::new(Vec::new());
    let changed = renderer
        .repaint_dirty(&model.snapshot(), &path, Mode::Auto, &[0])
        .unwrap();

    assert_eq!(changed.rows, [0, 1]);
}

#[test]
fn anchor_layout_changes_repaint_its_continuations() {
    let path = verified_path(Order::Logical);
    let mut model = TerminalModel::new(3, 20).unwrap();
    model.feed("\x1b[1;1Hשלום\x1b[2;1Henglish".as_bytes());
    let mut renderer = Renderer::new(Vec::new());
    renderer
        .repaint(&model.snapshot(), &path, Mode::Auto)
        .unwrap();

    model.feed(b"\x1b[1;1H\x1b[2Kanchor");
    let changed = renderer
        .repaint(&model.snapshot(), &path, Mode::Auto)
        .unwrap();

    assert_eq!(changed.rows, [0, 1]);
}

#[test]
fn visual_recovery_maps_the_reported_logical_column() {
    let mut model = TerminalModel::new(2, 10).unwrap();
    model.feed("דגבא".as_bytes());
    let snapshot = model.snapshot();
    let result = layout_row(
        &snapshot.physical_rows[0],
        pane(10),
        &verified_path(Order::Visual),
        Mode::Auto,
    );
    let map = result.coordinates.unwrap();

    assert_eq!(result.logical_text.as_deref(), Some("אבגד"));
    assert_eq!(map.visual_col(4), Some(6));
    assert_eq!(map.visual_col(1), Some(9));
}

#[test]
fn resize_recomputes_wrapped_row_maps_and_renderer_cursor() {
    let mut model = TerminalModel::new(4, 8).unwrap();
    model.feed("אבגדהוזחט".as_bytes());
    model.resize(4, 5).unwrap();
    let snapshot = model.snapshot();
    let results = layout_rows(
        &snapshot.physical_rows,
        pane(5),
        &verified_path(Order::Logical),
        Mode::Auto,
    );

    assert_eq!(results[0].coordinates.as_ref().unwrap().logical_start, 0);
    assert_eq!(results[0].coordinates.as_ref().unwrap().logical_end, 5);
    assert_eq!(results[1].coordinates.as_ref().unwrap().logical_start, 5);
    assert_eq!(results[1].coordinates.as_ref().unwrap().logical_end, 9);

    let mut renderer = Renderer::new(Vec::new());
    let summary = renderer
        .repaint(&snapshot, &verified_path(Order::Logical), Mode::Auto)
        .unwrap();
    assert_eq!(summary.cursor.row, 1);
    assert_eq!(summary.cursor.col, 1);
}

#[test]
fn cursor_only_vertical_movement_uses_the_destination_rows_map() {
    let path = verified_path(Order::Logical);
    let mut model = TerminalModel::new(3, 12).unwrap();
    model.feed("אבגד\x1b[2;1Hאבגד\x1b[1;5H".as_bytes());
    let mut renderer = Renderer::new(Vec::new());
    renderer
        .repaint(&model.snapshot(), &path, Mode::Auto)
        .unwrap();

    model.feed(b"\x1b[2;5H");
    let moved = renderer
        .repaint(&model.snapshot(), &path, Mode::Auto)
        .unwrap();

    assert!(moved.rows.is_empty());
    assert_eq!(moved.cursor.row, 1);
    assert_eq!(moved.cursor.col, 8);
}

#[test]
fn renderer_repaints_only_changed_rows_and_restores_mapped_caret() {
    let path = verified_path(Order::Logical);
    let mut model = TerminalModel::new(3, 12).unwrap();
    model.feed("אבגד".as_bytes());
    let mut renderer = Renderer::new(Vec::new());

    let first = renderer
        .repaint(&model.snapshot(), &path, Mode::Auto)
        .unwrap();
    assert_eq!(first.rows, [0, 1, 2]);
    assert_eq!(first.cursor.row, 0);
    assert_eq!(first.cursor.col, 8);

    let unchanged = renderer
        .repaint(&model.snapshot(), &path, Mode::Auto)
        .unwrap();
    assert!(unchanged.rows.is_empty());

    model.feed(b"\x1b[3;1Hlatin");
    let changed = renderer
        .repaint(&model.snapshot(), &path, Mode::Auto)
        .unwrap();
    assert_eq!(changed.rows, [2]);
    assert_eq!(changed.cursor.row, 2);
    assert_eq!(changed.cursor.col, 5);

    let output = String::from_utf8(renderer.into_inner()).unwrap();
    assert!(output.contains("\x1b[1;1H"));
    assert!(output.contains("\x1b[3;1H"));
    assert!(output.ends_with("\x1b[3;6H\x1b[?25h"));
}

#[test]
fn the_caret_stands_against_the_character_last_typed() {
    #[derive(serde::Deserialize)]
    struct Sample {
        typed: String,
        row: String,
        caret: u16,
    }

    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test/fixtures/punctuation-samples.json"
    );
    let samples: Vec<Sample> =
        serde_json::from_str(&std::fs::read_to_string(fixture).unwrap()).unwrap();
    for sample in &samples {
        let mut model = TerminalModel::new(2, 100).unwrap();
        model.feed(format!("\x1b[H{}\x1b[1;{}H", sample.row, sample.caret + 1).as_bytes());
        let snapshot = model.snapshot();
        assert_eq!(snapshot.cursor.col, sample.caret, "{}", sample.typed);

        let path = verified_path(Order::Visual);
        let result = layout_row(&snapshot.physical_rows[0], pane(100), &path, Mode::Auto);
        let mut renderer = Renderer::new(Vec::new());
        let got = renderer
            .repaint(&snapshot, &path, Mode::Auto)
            .unwrap()
            .cursor
            .col;
        let cell = |col: u16| {
            result
                .cells
                .get(usize::from(col))
                .map(|cell| cell.text.as_str())
                .unwrap_or_default()
        };

        // A trailing space paints nothing and still owes the caret a column, so
        // the character it stands against sits that many columns to its right.
        let stripped = sample.typed.trim_end_matches(' ');
        let blanks = (sample.typed.len() - stripped.len()) as u16;
        for step in 0..blanks {
            assert!(
                cell(got + step).trim().is_empty(),
                "typed {:?}: column {} carries {:?}, not the space typed",
                sample.typed,
                got + step,
                cell(got + step)
            );
        }
        let last = stripped.chars().next_back().unwrap();
        // Forward is right for a Latin character and left for the rest, which
        // bidi laid out right to left: the caret takes the cell itself there.
        let (against, expected) = if last.is_ascii_alphanumeric() {
            (got + blanks - 1, last)
        } else {
            (got + blanks, last)
        };
        assert_eq!(
            cell(against),
            expected.to_string(),
            "typed {:?} painted {:?}: the caret landed on {}",
            sample.typed,
            sample.row,
            got
        );
    }
    assert_eq!(samples.len(), 84);
}

fn verified_path(order: Order) -> ExecutionPath {
    ExecutionPath {
        order: Some(order),
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
