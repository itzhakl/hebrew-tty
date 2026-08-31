use hebrew_tty::classify::{Confidence, ExecutionPath, Order, Wrapping};
use hebrew_tty::config::Mode;
use hebrew_tty::layout::{layout_row, layout_rows, LayoutResult};
use hebrew_tty::terminal::{CellWidth, Color, PaneSpan, TerminalError, TerminalModel};
use serde::Deserialize;

fn row_text(model: &TerminalModel, row: usize) -> String {
    model.snapshot().physical_rows[row]
        .cells
        .iter()
        .filter(|cell| cell.width != CellWidth::Continuation)
        .map(|cell| cell.text.as_str())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn snapshots_styles_cursor_and_visibility() {
    let mut model = TerminalModel::new(3, 12).unwrap();
    model.feed(b"plain \x1b[1;3;4;38;2;1;2;3;48;5;4mX\x1b[0m\x1b[?25l");
    let snapshot = model.snapshot();
    let cell = &snapshot.physical_rows[0].cells[6];

    assert_eq!(snapshot.size.rows, 3);
    assert_eq!(snapshot.size.cols, 12);
    assert_eq!(cell.text, "X");
    assert!(cell.style.bold);
    assert!(cell.style.italic);
    assert!(cell.style.underline);
    assert_eq!(cell.style.foreground, Color::Rgb(1, 2, 3));
    assert_eq!(cell.style.background, Color::Indexed(4));
    assert!(!snapshot.cursor.visible);
    assert_eq!(model.cursor(), snapshot.cursor);
}

#[test]
fn preserves_wide_and_combining_cells() {
    let mut model = TerminalModel::new(2, 8).unwrap();
    model.feed("א\u{05b0}界".as_bytes());
    let row = &model.snapshot().physical_rows[0];

    assert_eq!(row.cells[0].text, "א\u{05b0}");
    assert_eq!(row.cells[0].width, CellWidth::Single);
    assert_eq!(row.cells[1].text, "界");
    assert_eq!(row.cells[1].width, CellWidth::Wide);
    assert_eq!(row.cells[2].width, CellWidth::Continuation);
}

#[test]
fn preserves_hard_and_soft_physical_row_boundaries() {
    let mut model = TerminalModel::new(4, 5).unwrap();
    model.feed(b"abcdeZ\r\nnext");
    let snapshot = model.snapshot();

    assert_eq!(row_text(&model, 0), "abcde");
    assert_eq!(row_text(&model, 1), "Z");
    assert_eq!(row_text(&model, 2), "next");
    assert!(snapshot.physical_rows[0].soft_wrapped);
    assert!(!snapshot.physical_rows[1].soft_wrapped);
}

#[test]
fn pane_spans_exclude_real_dividers_and_reject_false_positives() {
    let mut model = TerminalModel::new(10, 12).unwrap();
    for row in 0..9 {
        model.feed(format!("\x1b[{};1HL│R", row + 1).as_bytes());
    }
    let snapshot = model.snapshot();
    assert_eq!(snapshot.pane_spans.len(), 2);
    assert_eq!(snapshot.pane_spans[0].start_col, 0);
    assert_eq!(snapshot.pane_spans[0].end_col, 1);
    assert_eq!(snapshot.pane_spans[1].start_col, 2);
    assert_eq!(snapshot.pane_spans[1].end_col, 12);

    let mut short = TerminalModel::new(7, 12).unwrap();
    for row in 0..7 {
        short.feed(format!("\x1b[{};1HL│R", row + 1).as_bytes());
    }
    assert_eq!(short.snapshot().pane_spans.len(), 1);

    let mut sparse = TerminalModel::new(10, 12).unwrap();
    for row in 0..8 {
        sparse.feed(format!("\x1b[{};1HL│R", row + 1).as_bytes());
    }
    assert_eq!(sparse.snapshot().pane_spans.len(), 1);
}

#[test]
fn dirty_rows_are_final_state_diffs_and_ignore_cursor_only_moves() {
    let mut model = TerminalModel::new(3, 8).unwrap();
    assert_eq!(dirty_indices(&mut model), vec![0, 1, 2]);
    model.feed(b"abc\x1b[1D\x1b[1C");
    assert_eq!(dirty_indices(&mut model), vec![0]);
    model.feed(b"\x1b[2D\x1b[2C");
    assert!(model.take_dirty_rows().is_empty());
    model.feed(b"\x1b[1DZZ");
    model.feed(b"\x1b[2Dcc");
    let dirty = model.take_dirty_rows();
    assert_eq!(
        dirty.iter().map(|row| row.row_index).collect::<Vec<_>>(),
        vec![0]
    );
    assert_eq!(dirty[0].row, model.snapshot().physical_rows[0]);
}

#[test]
fn primary_resize_reflows_and_preserves_hebrew_codepoint_order() {
    let mut model = TerminalModel::new(4, 8).unwrap();
    model.feed("אבגדהוזחט".as_bytes());
    model.take_dirty_rows();
    model.resize(4, 5).unwrap();
    assert_eq!(row_text(&model, 0), "אבגדה");
    assert_eq!(row_text(&model, 1), "וזחט");
    assert_eq!(dirty_indices(&mut model), vec![0, 1, 2, 3]);
    model.resize(4, 10).unwrap();
    assert_eq!(row_text(&model, 0), "אבגדהוזחט");
}

#[test]
fn alternate_screen_is_isolated_and_resize_keeps_physical_placement() {
    let mut model = TerminalModel::new(4, 8).unwrap();
    model.feed("אבגדהוזחט".as_bytes());
    model.feed(b"\x1b[?1049h\x1b[3;4HALT");
    assert!(model.snapshot().alternate_screen);
    assert_eq!(model.snapshot().physical_rows[2].cells[3].text, "A");
    model.resize(5, 5).unwrap();
    assert_eq!(model.snapshot().physical_rows[2].cells[3].text, "A");
    assert_eq!(model.snapshot().physical_rows[2].cells[4].text, "L");
    model.feed(b"\x1b[?1049l");
    assert!(!model.snapshot().alternate_screen);
    assert_eq!(row_text(&model, 0), "אבגדה");
    assert_eq!(row_text(&model, 1), "וזחט");
}

#[test]
fn split_alternate_mode_reflows_primary_at_resized_width() {
    let mut model = TerminalModel::new(4, 8).unwrap();
    model.feed("אבגדהוזחט".as_bytes());
    for byte in b"\x1b[?1049h" {
        model.feed(std::slice::from_ref(byte));
    }
    model.feed(b"\x1b[3;4HALT");
    model.resize(5, 5).unwrap();
    assert_eq!(model.snapshot().physical_rows[2].cells[3].text, "A");
    assert_eq!(model.snapshot().physical_rows[2].cells[4].text, "L");
    for chunk in b"\x1b[?1049l".chunks(2) {
        model.feed(chunk);
    }
    assert!(!model.snapshot().alternate_screen);
    assert_eq!(row_text(&model, 0), "אבגדה");
    assert_eq!(row_text(&model, 1), "וזחט");
    assert_eq!(row_text(&model, 2), "");
}

#[test]
fn pane_span_changes_dirty_every_visible_row() {
    let mut model = TerminalModel::new(10, 6).unwrap();
    model.take_dirty_rows();
    for row in 0..9 {
        model.feed(format!("\x1b[{};2H│", row + 1).as_bytes());
    }
    assert_eq!(dirty_indices(&mut model), (0..10).collect::<Vec<_>>());
}

#[test]
fn wide_cells_at_right_edge_never_leave_dangling_continuations() {
    let mut model = TerminalModel::new(2, 4).unwrap();
    model.feed(b"abc");
    model.feed("界".as_bytes());
    assert_wide_cells_are_complete(&model);

    model.feed(b"\x1b[?1049h\x1b[1;3H");
    model.feed("界".as_bytes());
    model.resize(2, 3).unwrap();
    assert_wide_cells_are_complete(&model);
    assert_eq!(
        model.snapshot().physical_rows[0].cells[2].width,
        CellWidth::Empty
    );
}

#[test]
fn resize_rejects_zero_without_mutation() {
    let mut model = TerminalModel::new(2, 8).unwrap();
    model.feed(b"abcd");
    let before = model.snapshot();
    assert_eq!(model.resize(0, 8), Err(TerminalError::ZeroSize));
    assert_eq!(model.snapshot(), before);
    model.resize(2, 4).unwrap();
    assert_eq!(row_text(&model, 0), "abcd");
}

#[test]
fn scroll_regions_affect_only_the_region() {
    let mut model = TerminalModel::new(5, 8).unwrap();
    model.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
    model.feed(b"\x1b[2;4r\x1b[4;1H\n");
    assert_eq!(row_text(&model, 0), "one");
    assert_eq!(row_text(&model, 1), "three");
    assert_eq!(row_text(&model, 2), "four");
    assert_eq!(row_text(&model, 4), "five");
}

#[test]
fn auto_mode_preserves_unknown_rows_exactly() {
    let mut model = TerminalModel::new(2, 8).unwrap();
    model.feed("אבגד".as_bytes());
    let row = &model.snapshot().physical_rows[0];
    let result = layout_row(row, pane(8), &unknown_path(), Mode::Auto);

    assert_eq!(result.cells, row.cells);
    assert!(!result.transformed);
    assert!(!result.right_aligned);
}

#[test]
fn logical_rows_are_reordered_mirrored_and_right_aligned() {
    let mut model = TerminalModel::new(2, 20).unwrap();
    model.feed("שלום (בדיקה)".as_bytes());
    let snapshot = model.snapshot();
    let result = layout_row(
        &snapshot.physical_rows[0],
        pane(20),
        &verified_path(Order::Logical),
        Mode::Auto,
    );

    assert_eq!(layout_text(&result), "(הקידב) םולש");
    assert!(result.right_aligned);
    assert_eq!(first_nonempty(&result), 8);
    assert_eq!(result.logical_text.as_deref(), Some("שלום (בדיקה)"));
}

#[test]
fn visual_rows_recover_verified_mixed_content_before_layout() {
    for (logical, painted) in [
        ("❯\u{a0}שלום עולם", "❯\u{a0}םלוע םולש"),
        ("❯\u{a0}שלום, מה נשמע.", "❯\u{a0}.עמשנ המ ,םולש"),
        (
            "❯\u{a0}קובץ src/auth.ts שורה 42",
            "❯\u{a0}42 הרוש src/auth.ts ץבוק",
        ),
    ] {
        let mut logical_model = TerminalModel::new(2, 40).unwrap();
        logical_model.feed(logical.as_bytes());
        let logical_snapshot = logical_model.snapshot();
        let expected = layout_row(
            &logical_snapshot.physical_rows[0],
            pane(40),
            &verified_path(Order::Logical),
            Mode::Auto,
        );

        let mut visual_model = TerminalModel::new(2, 40).unwrap();
        visual_model.feed(painted.as_bytes());
        let visual_snapshot = visual_model.snapshot();
        let actual = layout_row(
            &visual_snapshot.physical_rows[0],
            pane(40),
            &verified_path(Order::Visual),
            Mode::Auto,
        );

        assert_eq!(actual.cells, expected.cells, "{logical}");
        assert_eq!(actual.logical_text.as_deref(), Some(logical), "{logical}");
    }
}

#[test]
fn post_bidi_wrapping_is_recovered_before_per_row_resolution() {
    let mut model = TerminalModel::new(3, 4).unwrap();
    model.feed("חזוהדגבא".as_bytes());
    let snapshot = model.snapshot();
    assert_eq!(row_text(&model, 0), "חזוה");
    assert_eq!(row_text(&model, 1), "דגבא");
    assert!(snapshot.physical_rows[0].soft_wrapped);

    let results = layout_rows(
        &snapshot.physical_rows[..2],
        pane(4),
        &verified_path(Order::Visual),
        Mode::Auto,
    );

    assert_eq!(
        results.iter().map(layout_text).collect::<Vec<_>>(),
        ["דגבא", "חזוה"]
    );
    assert_eq!(results[0].logical_text.as_deref(), Some("אבגדהוזח"));
}

#[test]
fn hard_prose_rows_inherit_the_anchor_layout() {
    let mut model = TerminalModel::new(4, 20).unwrap();
    model.feed("\x1b[1;1Hשלום\x1b[2;1Henglish\x1b[3;1Habc אב".as_bytes());
    let snapshot = model.snapshot();
    let results = layout_rows(
        &snapshot.physical_rows,
        pane(20),
        &verified_path(Order::Logical),
        Mode::Auto,
    );

    assert!(results[0].right_aligned);
    assert!(results[1].right_aligned);
    assert_eq!(layout_text(&results[1]), "english");
    assert_eq!(first_nonempty(&results[1]), 13);
    assert_eq!(results[1].cells[19].text, "h");
    assert!(results[2].right_aligned);
    assert_eq!(results[2].cells[19].text, "c");
}

#[test]
fn latin_opening_hebrew_majority_anchor_uses_preferred_base() {
    let mut model = TerminalModel::new(4, 24).unwrap();
    model.feed(b"\x1b[1;1Habc \xd7\x90\xd7\x91\xd7\x92\xd7\x93\xd7\x94\x1b[2;1Henglish");
    let snapshot = model.snapshot();
    let results = layout_rows(
        &snapshot.physical_rows,
        pane(24),
        &verified_path(Order::Logical),
        Mode::Auto,
    );

    assert!(results[0].right_aligned);
    assert_eq!(first_nonempty(&results[0]), 15);
    assert!(results[1].right_aligned);
    assert_eq!(layout_text(&results[1]), "english");
    assert_eq!(first_nonempty(&results[1]), 17);
}

#[test]
fn unstyled_indented_code_breaks_inheritance_before_trimming() {
    let mut model = TerminalModel::new(4, 24).unwrap();
    model.feed("\x1b[1;1Hשלום\x1b[2;1H    const x = 1;\x1b[3;1Henglish".as_bytes());
    let snapshot = model.snapshot();
    let results = layout_rows(
        &snapshot.physical_rows,
        pane(24),
        &verified_path(Order::Logical),
        Mode::Auto,
    );

    assert_eq!(results[1].cells, snapshot.physical_rows[1].cells);
    assert_eq!(layout_text(&results[1]), row_text(&model, 1));
    assert_eq!(first_nonempty(&results[2]), 0);
    assert!(!results[2].right_aligned);
}

#[test]
fn terminal_tabs_mark_code_boundary_without_capturing_padding() {
    let mut model = TerminalModel::new(4, 32).unwrap();
    model.feed(b"\x1b[1;1H\xd7\xa9\xd7\x9c\xd7\x95\xd7\x9d\x1b[2;1H\tconst x = 1;\x1b[3;1Henglish");
    let snapshot = model.snapshot();
    assert!(snapshot.physical_rows[1].cells[..8]
        .iter()
        .all(|cell| cell.width == CellWidth::Empty));
    assert_eq!(snapshot.physical_rows[1].cells[8].text, "c");
    let results = layout_rows(
        &snapshot.physical_rows,
        pane(32),
        &verified_path(Order::Logical),
        Mode::Auto,
    );

    assert_eq!(results[1].cells, snapshot.physical_rows[1].cells);
    assert_eq!(first_nonempty(&results[2]), 0);
    assert!(!results[2].right_aligned);
}

#[test]
fn right_alignment_padding_beyond_code_indent_keeps_prose_inheritance() {
    let mut model = TerminalModel::new(3, 24).unwrap();
    model.feed("\x1b[1;1Hשלום\x1b[2;1H            english\x1b[3;1Hlater".as_bytes());
    let results = layout_rows(
        &model.snapshot().physical_rows,
        pane(24),
        &verified_path(Order::Logical),
        Mode::Auto,
    );

    assert!(results[1].right_aligned);
    assert!(first_nonempty(&results[1]) > 4);
    assert!(results[2].right_aligned);
}

#[test]
fn visual_mixed_continuation_recovers_with_the_anchor_base() {
    let mut logical_model = TerminalModel::new(3, 30).unwrap();
    logical_model.feed("\x1b[1;1Hשלום\x1b[2;1Habc אב".as_bytes());
    let logical_snapshot = logical_model.snapshot();
    let expected = layout_rows(
        &logical_snapshot.physical_rows,
        pane(30),
        &verified_path(Order::Logical),
        Mode::Auto,
    );

    let mut visual_model = TerminalModel::new(3, 30).unwrap();
    visual_model.feed(
        format!(
            "\x1b[1;1H{}\x1b[2;1H{}",
            layout_text(&expected[0]).trim_start(),
            layout_text(&expected[1]).trim_start()
        )
        .as_bytes(),
    );
    let visual_snapshot = visual_model.snapshot();
    let actual = layout_rows(
        &visual_snapshot.physical_rows,
        pane(30),
        &verified_path(Order::Visual),
        Mode::Auto,
    );

    assert_eq!(actual[0].cells, expected[0].cells);
    assert_eq!(actual[1].cells, expected[1].cells);
    assert_eq!(actual[1].logical_text.as_deref(), Some("abc אב"));
    assert!(actual[1].right_aligned);
}

#[test]
fn hard_continuations_keep_glyph_metadata_and_order() {
    let mut model = TerminalModel::new(3, 24).unwrap();
    model.feed(
        "\x1b[1;1Hשלום\x1b[2;1H\x1b[31meng\x1b[39m\x1b]8;;https://example.com\x07lish\x1b]8;;\x07"
            .as_bytes(),
    );
    let snapshot = model.snapshot();
    let results = layout_rows(
        &snapshot.physical_rows,
        pane(24),
        &verified_path(Order::Logical),
        Mode::Auto,
    );

    assert_eq!(layout_text(&results[1]), "english");
    assert!(results[1].right_aligned);
    assert_eq!(results[1].cells[17].text, "e");
    assert_eq!(results[1].cells[17].style.foreground, Color::Indexed(1));
    assert_eq!(
        results[1].cells[20].hyperlink.as_deref(),
        Some("https://example.com")
    );
}

#[test]
fn later_soft_wrap_fragments_do_not_reset_lexical_paragraphs() {
    for fragment in ["https://example.com", "- item", "❯ prompt", "---"] {
        let mut model = TerminalModel::new(4, 40).unwrap();
        model.feed(format!("\x1b[1;1Hשלום\x1b[2;1H{fragment}\x1b[3;1Henglish").as_bytes());
        let mut rows = model.snapshot().physical_rows;
        rows[0].soft_wrapped = true;
        let results = layout_rows(&rows, pane(40), &verified_path(Order::Logical), Mode::Auto);

        assert!(results[2].right_aligned, "{fragment:?}");
        assert!(first_nonempty(&results[2]) > 0, "{fragment:?}");
    }
}

#[test]
fn recognizable_non_prose_rows_break_paragraph_inheritance() {
    let boundaries = [
        "",
        "- item",
        "1. item",
        "```",
        "https://example.com",
        "|a|",
        "---",
        "❯ ",
        "\x1b]8;;https://example.com\x07linked\x1b]8;;\x07",
        "\x1b[48;5;4mcode                    \x1b[0m",
        "\x1b[7mui                      \x1b[0m",
    ];
    for boundary in boundaries {
        let mut model = TerminalModel::new(4, 24).unwrap();
        model.feed(format!("\x1b[1;1Hשלום\x1b[2;1H{boundary}\x1b[3;1Henglish").as_bytes());
        let snapshot = model.snapshot();
        let results = layout_rows(
            &snapshot.physical_rows,
            pane(24),
            &verified_path(Order::Logical),
            Mode::Auto,
        );
        assert_eq!(first_nonempty(&results[2]), 0, "{boundary:?}");
        assert!(!results[2].right_aligned, "{boundary:?}");
    }
}

#[test]
fn viewport_top_is_an_independent_paragraph_anchor() {
    let mut model = TerminalModel::new(3, 20).unwrap();
    model.feed("\x1b[1;1Hשלום\x1b[2;1Henglish".as_bytes());
    let snapshot = model.snapshot();
    let results = layout_rows(
        &snapshot.physical_rows[1..],
        pane(20),
        &verified_path(Order::Logical),
        Mode::Auto,
    );

    assert_eq!(first_nonempty(&results[0]), 0);
    assert!(!results[0].right_aligned);
}

#[test]
fn paragraph_layout_is_pane_local() {
    let mut model = TerminalModel::new(3, 21).unwrap();
    model.feed("\x1b[1;1Hשלום\x1b[2;1Henglish\x1b[1;12Hleft\x1b[2;12Hright".as_bytes());
    let snapshot = model.snapshot();
    let left = layout_rows(
        &snapshot.physical_rows,
        PaneSpan {
            start_col: 0,
            end_col: 10,
        },
        &verified_path(Order::Logical),
        Mode::Auto,
    );
    let right = layout_rows(
        &snapshot.physical_rows,
        PaneSpan {
            start_col: 11,
            end_col: 21,
        },
        &verified_path(Order::Logical),
        Mode::Auto,
    );

    assert!(left[1].right_aligned);
    assert_eq!(first_nonempty_from(&left[1], 0), 3);
    assert_eq!(first_nonempty_from(&right[1], 11), 11);
}

#[test]
fn unknown_path_preserves_paragraph_candidates_exactly() {
    let mut model = TerminalModel::new(3, 20).unwrap();
    model.feed("\x1b[1;1Hשלום\x1b[2;1Henglish".as_bytes());
    let snapshot = model.snapshot();
    let results = layout_rows(
        &snapshot.physical_rows,
        pane(20),
        &unknown_path(),
        Mode::Auto,
    );

    assert_eq!(results[0].cells, snapshot.physical_rows[0].cells);
    assert_eq!(results[1].cells, snapshot.physical_rows[1].cells);
    assert!(!results[1].transformed);
}

#[test]
fn wrapped_pane_layout_preserves_each_physical_rows_other_panes() {
    let mut model = TerminalModel::new(2, 8).unwrap();
    model.feed("\x1b[1;1HA│\x1b[1;5Hחזוה\x1b[2;1HB│\x1b[2;5Hדגבא".as_bytes());
    let mut rows = model.snapshot().physical_rows;
    rows[0].soft_wrapped = true;
    let results = layout_rows(
        &rows,
        PaneSpan {
            start_col: 4,
            end_col: 8,
        },
        &verified_path(Order::Visual),
        Mode::Auto,
    );

    assert_eq!(results[0].cells[0].text, "A");
    assert_eq!(results[1].cells[0].text, "B");
    assert_eq!(results[0].cells[1].text, "│");
    assert_eq!(results[1].cells[1].text, "│");
}

#[test]
fn pane_alignment_and_table_cell_layout_keep_rules_fixed() {
    let mut split = TerminalModel::new(2, 12).unwrap();
    split.feed("L│שלום".as_bytes());
    let split_snapshot = split.snapshot();
    let result = layout_row(
        &split_snapshot.physical_rows[0],
        PaneSpan {
            start_col: 2,
            end_col: 12,
        },
        &verified_path(Order::Logical),
        Mode::Auto,
    );
    assert_eq!(result.cells[1].text, "│");
    assert_eq!(first_nonempty_from(&result, 2), 8);
    assert_eq!(layout_text(&result), "L│םולש");

    let mut table = TerminalModel::new(2, 12).unwrap();
    table.feed("│שלום│".as_bytes());
    let table_snapshot = table.snapshot();
    let table_result = layout_row(
        &table_snapshot.physical_rows[0],
        pane(12),
        &verified_path(Order::Logical),
        Mode::Auto,
    );
    assert_eq!(layout_text(&table_result), "│םולש│");
    assert!(!table_result.right_aligned);
}

#[test]
fn layout_falls_back_when_a_pane_cannot_hold_a_wide_grapheme() {
    let mut model = TerminalModel::new(2, 8).unwrap();
    model.feed("א界".as_bytes());
    let snapshot = model.snapshot();
    let row = &snapshot.physical_rows[0];
    let result = layout_row(
        row,
        PaneSpan {
            start_col: 1,
            end_col: 2,
        },
        &verified_path(Order::Logical),
        Mode::Auto,
    );

    assert_eq!(result.cells, row.cells);
    assert!(!result.transformed);
}

#[test]
fn reordered_glyphs_keep_their_original_style() {
    let mut model = TerminalModel::new(2, 8).unwrap();
    model.feed("\x1b[31;44mא\x1b[39mבגד    ".as_bytes());
    let snapshot = model.snapshot();
    let result = layout_row(
        &snapshot.physical_rows[0],
        pane(8),
        &verified_path(Order::Logical),
        Mode::Auto,
    );
    let aleph = result.cells.iter().find(|cell| cell.text == "א").unwrap();
    assert_eq!(aleph.style.foreground, Color::Indexed(1));
    assert!(result
        .cells
        .iter()
        .all(|cell| cell.style.background == Color::Indexed(4)));
}

fn verified_path(order: Order) -> ExecutionPath {
    ExecutionPath {
        order: Some(order),
        wrapping: Some(Wrapping::PostBidi),
        confidence: Confidence::Verified,
        evidence: Vec::new(),
    }
}

fn unknown_path() -> ExecutionPath {
    ExecutionPath {
        order: None,
        wrapping: None,
        confidence: Confidence::Unknown,
        evidence: Vec::new(),
    }
}

fn pane(cols: u16) -> PaneSpan {
    PaneSpan {
        start_col: 0,
        end_col: cols,
    }
}

fn layout_text(result: &LayoutResult) -> String {
    result
        .cells
        .iter()
        .filter(|cell| cell.width != CellWidth::Continuation)
        .map(|cell| cell.text.as_str())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn first_nonempty(result: &LayoutResult) -> usize {
    first_nonempty_from(result, 0)
}

fn first_nonempty_from(result: &LayoutResult, start: usize) -> usize {
    result.cells[start..]
        .iter()
        .position(|cell| !cell.text.is_empty())
        .map(|offset| start + offset)
        .unwrap()
}

#[test]
fn fixtures_use_the_versioned_schema() {
    for fixture in [
        include_str!("../test/fixtures/terminal-proxy/screens/styles.json"),
        include_str!("../test/fixtures/terminal-proxy/screens/layout.json"),
        include_str!("../test/fixtures/terminal-proxy/screens/resize.json"),
    ] {
        run_fixture(serde_json::from_str(fixture).unwrap());
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema_version: u8,
    name: String,
    initial_size: FixtureSize,
    events: Vec<Event>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureSize {
    rows: u16,
    cols: u16,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Feed { hex: String },
    Resize { rows: u16, cols: u16 },
    Checkpoint { expected: Expected },
    DrainDirty { expected_indices: Vec<u16> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    rows: Vec<String>,
    cursor: ExpectedCursor,
    alternate_screen: bool,
    pane_spans: Vec<PaneSpanFixture>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedCursor {
    row: u16,
    col: u16,
    visible: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaneSpanFixture {
    start_col: u16,
    end_col: u16,
}

fn run_fixture(fixture: Fixture) {
    assert_eq!(fixture.schema_version, 1, "{}", fixture.name);
    let mut model =
        TerminalModel::new(fixture.initial_size.rows, fixture.initial_size.cols).unwrap();
    for event in fixture.events {
        match event {
            Event::Feed { hex } => model.feed(&decode_hex(&hex)),
            Event::Resize { rows, cols } => model.resize(rows, cols).unwrap(),
            Event::DrainDirty { expected_indices } => assert_eq!(
                dirty_indices(&mut model),
                expected_indices,
                "{}",
                fixture.name
            ),
            Event::Checkpoint { expected } => {
                let snapshot = model.snapshot();
                let rows = (0..snapshot.physical_rows.len())
                    .map(|row| row_text(&model, row))
                    .collect::<Vec<_>>();
                assert_eq!(rows, expected.rows, "{}", fixture.name);
                assert_eq!(
                    (
                        snapshot.cursor.row,
                        snapshot.cursor.col,
                        snapshot.cursor.visible
                    ),
                    (
                        expected.cursor.row,
                        expected.cursor.col,
                        expected.cursor.visible
                    ),
                    "{}",
                    fixture.name
                );
                assert_eq!(
                    snapshot.alternate_screen, expected.alternate_screen,
                    "{}",
                    fixture.name
                );
                assert_eq!(
                    snapshot.pane_spans,
                    expected
                        .pane_spans
                        .into_iter()
                        .map(|span| PaneSpan {
                            start_col: span.start_col,
                            end_col: span.end_col
                        })
                        .collect::<Vec<_>>(),
                    "{}",
                    fixture.name
                );
            }
        }
    }
}

fn dirty_indices(model: &mut TerminalModel) -> Vec<u16> {
    model
        .take_dirty_rows()
        .into_iter()
        .map(|row| row.row_index)
        .collect()
}

fn assert_wide_cells_are_complete(model: &TerminalModel) {
    for row in &model.snapshot().physical_rows {
        for (col, cell) in row.cells.iter().enumerate() {
            if cell.width == CellWidth::Wide {
                assert_eq!(
                    row.cells.get(col + 1).map(|next| next.width),
                    Some(CellWidth::Continuation),
                    "{row:?}"
                );
            }
            if cell.width == CellWidth::Continuation {
                assert!(col > 0);
                assert_eq!(row.cells[col - 1].width, CellWidth::Wide);
            }
        }
    }
}

fn decode_hex(value: &str) -> Vec<u8> {
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty(), "fixture hex must have an even length");
    pairs
        .iter()
        .map(|pair| {
            let pair = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(pair, 16).unwrap()
        })
        .collect()
}
