//! A recorded session, not a hand-written stream: the first 45 pty reads of a
//! real Claude Code run, up to and past the frame that used to smear.

use hebrew_tty::classify::{Confidence, ExecutionPath, Order, Wrapping};
use hebrew_tty::config::Mode;
use hebrew_tty::layout::layout_rows;
use hebrew_tty::relay::Transform;
use hebrew_tty::terminal::TerminalModel;
use hebrew_tty::trace::{parse_trace, TraceRecord};

const ROWS: u16 = 68;
const COLS: u16 = 132;

fn verified() -> ExecutionPath {
    ExecutionPath {
        order: Some(Order::Visual),
        wrapping: Some(Wrapping::PostBidi),
        confidence: Confidence::Verified,
        evidence: Vec::new(),
    }
}

/// What the proxy means the screen to hold for the child's current screen.
fn intended(model: &TerminalModel) -> Vec<String> {
    let snapshot = model.snapshot();
    let path = verified();
    let mut rows = snapshot.physical_rows.clone();
    for &pane in &snapshot.pane_spans {
        let results = layout_rows(&snapshot.physical_rows, pane, &path, Mode::Auto);
        for (index, result) in results.into_iter().enumerate() {
            let start = usize::from(pane.start_col).min(rows[index].cells.len());
            let end = usize::from(pane.end_col).min(rows[index].cells.len());
            rows[index].cells[start..end].clone_from_slice(&result.cells[start..end]);
        }
    }
    rows.iter()
        .map(|row| {
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
        })
        .collect()
}

#[test]
fn a_recorded_session_paints_the_screen_the_proxy_means_to_paint() {
    let records = parse_trace(include_str!(
        "../test/fixtures/terminal-proxy/traces/synchronized-update-smear.trace"
    ));
    let mut transform = Transform::new(Vec::new(), ROWS, COLS, verified(), Mode::Auto).unwrap();
    let mut real = TerminalModel::new(ROWS, COLS).unwrap();
    let mut chunk = 0usize;

    for record in &records {
        match record {
            TraceRecord::Input(bytes) => {
                transform.feed(bytes).unwrap();
                let painted = std::mem::take(transform.writer_mut());
                real.feed_untracked(&painted);
                chunk += 1;
                let meant = intended(transform.model());
                let height = meant.len() as u16;
                let saw = (0..height)
                    .map(|row| real.row_text(row))
                    .collect::<Vec<_>>();
                for row in 0..usize::from(height) {
                    assert_eq!(
                        meant[row].trim_end(),
                        saw[row].trim_end(),
                        "chunk {chunk} row {row}"
                    );
                }
            }
            TraceRecord::Output(_) => {}
            TraceRecord::Resize { rows, cols } => {
                transform.resize(*rows, *cols).unwrap();
                let painted = std::mem::take(transform.writer_mut());
                real.feed_untracked(&painted);
                real.resize(*rows, *cols).unwrap();
            }
        }
    }

    assert_eq!(chunk, 45, "the fixture lost its reads");
}
