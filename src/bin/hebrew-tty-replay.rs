#![forbid(unsafe_code)]

//! Replays a `HEBREW_TTY_TRACE` recording: the child's own bytes into one
//! model, the bytes the real terminal received into another, and reports the
//! first record where the two screens stop agreeing.

use std::error::Error;

use hebrew_tty::classify::{Confidence, ExecutionPath, Order, Wrapping};
use hebrew_tty::config::Mode;
use hebrew_tty::layout::layout_rows;
use hebrew_tty::relay::Transform;
use hebrew_tty::terminal::{PhysicalRowSnapshot, TerminalModel};
use hebrew_tty::trace::{parse_trace, TraceRecord};

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

fn screen(model: &TerminalModel) -> Vec<String> {
    model.snapshot().physical_rows.iter().map(text).collect()
}

/// What the proxy would paint for the child's current screen.
fn intended(model: &TerminalModel) -> Vec<String> {
    let snapshot = model.snapshot();
    let path = ExecutionPath {
        order: Some(Order::Visual),
        wrapping: Some(Wrapping::PostBidi),
        confidence: Confidence::Verified,
        evidence: Vec::new(),
    };
    let mut rows = snapshot.physical_rows.clone();
    for &pane in &snapshot.pane_spans {
        let results = layout_rows(&snapshot.physical_rows, pane, &path, Mode::Auto);
        for (index, result) in results.into_iter().enumerate() {
            let start = usize::from(pane.start_col).min(rows[index].cells.len());
            let end = usize::from(pane.end_col).min(rows[index].cells.len());
            rows[index].cells[start..end].clone_from_slice(&result.cells[start..end]);
        }
    }
    rows.iter().map(text).collect()
}

/// Re-runs the recorded child stream through the current relay and reports
/// where the screen it paints stops matching the screen it means to paint.
fn rerun(records: &[TraceRecord], rows: u16, cols: u16) -> Result<(), Box<dyn Error>> {
    let mut transform = Transform::new(Vec::new(), rows, cols, verified(), Mode::Auto)?;
    let mut real = TerminalModel::new(rows, cols)?;
    let mut chunk_index = 0usize;
    let mut divergences = 0usize;
    for record in records {
        match record {
            TraceRecord::Input(bytes) => {
                transform.feed(bytes)?;
                let painted = std::mem::take(transform.writer_mut());
                real.feed_untracked(&painted);
                chunk_index += 1;
                let meant = intended(transform.model());
                let saw = (0..rows).map(|row| real.row_text(row)).collect::<Vec<_>>();
                let differing = meant
                    .iter()
                    .zip(saw.iter())
                    .enumerate()
                    .filter(|(_, (a, b))| a.trim_end() != b.trim_end())
                    .map(|(row, _)| row)
                    .collect::<Vec<_>>();
                if !differing.is_empty() {
                    divergences += 1;
                    if divergences == 1 {
                        println!("first divergence at chunk {chunk_index}: rows {differing:?}");
                        for &row in differing.iter().take(4) {
                            println!("  meant |{}\n  saw   |{}", meant[row], saw[row]);
                        }
                    }
                }
            }
            TraceRecord::Output(_) => {}
            TraceRecord::Resize { rows, cols } => {
                transform.resize(*rows, *cols)?;
                let painted = std::mem::take(transform.writer_mut());
                real.feed_untracked(&painted);
                real.resize(*rows, *cols)?;
            }
        }
    }
    println!("chunks: {chunk_index}, chunks with a divergence: {divergences}");
    Ok(())
}

fn verified() -> ExecutionPath {
    ExecutionPath {
        order: Some(Order::Visual),
        wrapping: Some(Wrapping::PostBidi),
        confidence: Confidence::Verified,
        evidence: Vec::new(),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .ok_or("usage: hebrew-tty-replay <trace> [rows] [cols]")?;
    let rows = args
        .next()
        .and_then(|value| value.to_str()?.parse().ok())
        .unwrap_or(50u16);
    let cols = args
        .next()
        .and_then(|value| value.to_str()?.parse().ok())
        .unwrap_or(120u16);

    let records = parse_trace(&std::fs::read_to_string(&path)?);
    if std::env::var("REPLAY_RERUN").is_ok() {
        return rerun(&records, rows, cols);
    }
    let mut child = TerminalModel::new(rows, cols)?;
    let mut real = TerminalModel::new(rows, cols)?;
    let mut divergence = None;

    let mut rows_diverged: Option<(usize, Vec<usize>)> = None;

    let watch = std::env::var("REPLAY_WATCH")
        .ok()
        .and_then(|value| value.parse::<u16>().ok());
    let mut watched = String::new();
    let split_at = std::env::var("REPLAY_SPLIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok());
    let stop = std::env::var("REPLAY_STOP")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(usize::MAX);
    for (index, record) in records.iter().enumerate() {
        if index > stop {
            break;
        }
        if let (Some(split), TraceRecord::Input(bytes)) = (split_at, record) {
            if index == split {
                for (offset, byte) in bytes.iter().enumerate() {
                    child.feed_untracked(&[*byte]);
                    real.feed_untracked(&[*byte]);
                    if child.cursor() != real.cursor() || child.row_text(64) != real.row_text(64) {
                        println!(
                            "split {split} offset {offset} byte {byte:#04x}\n  child {:?} |{}\n  real  {:?} |{}",
                            child.cursor(),
                            child.row_text(64),
                            real.cursor(),
                            real.row_text(64)
                        );
                        return Ok(());
                    }
                }
                println!("split {split}: no divergence inside the record");
                return Ok(());
            }
        }
        match record {
            TraceRecord::Input(bytes) => child.feed_untracked(bytes),
            TraceRecord::Output(bytes) => real.feed_untracked(bytes),
            TraceRecord::Resize { rows, cols } => {
                child.resize(*rows, *cols)?;
                real.resize(*rows, *cols)?;
            }
        }
        // A repaint spans many Output records and only puts the cursor back at
        // its end, so the streams stand in the same place at the last Output
        // record before the child speaks again.
        let aligned = matches!(record, TraceRecord::Output(_))
            && !matches!(records.get(index + 1), Some(TraceRecord::Output(_)));
        if let Some(row) = watch {
            let now = if std::env::var("REPLAY_SIDE").as_deref() == Ok("child") {
                child.row_text(row)
            } else {
                real.row_text(row)
            };
            if now != watched {
                let tag = match record {
                    TraceRecord::Input(_) => "child",
                    TraceRecord::Output(_) => "ours",
                    TraceRecord::Resize { .. } => "resize",
                };
                println!("{index} {tag} row {row} -> |{now}");
                watched = now;
            }
        }
        if divergence.is_none() && aligned {
            let (left, right) = (child.cursor(), real.cursor());
            if left.row != right.row {
                divergence = Some((index, left.row, right.row));
            }
        }
        let in_window = std::env::var("REPLAY_FROM")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .is_some_and(|from| index >= from);
        if rows_diverged.is_none() && aligned && (in_window || index % 200 == 0) {
            let (left, right) = (intended(&child), screen(&real));
            let differing = left
                .iter()
                .zip(right.iter())
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .map(|(row, _)| row)
                .collect::<Vec<_>>();
            if !differing.is_empty() {
                for &row in &differing {
                    println!(
                        "record {index} row {row}\n  meant |{}\n  saw   |{}",
                        left[row], right[row]
                    );
                }
                rows_diverged = Some((index, differing));
            }
        }
    }

    println!(
        "child cursor {:?} real cursor {:?}",
        child.cursor(),
        real.cursor()
    );
    println!("records: {}", records.len());
    match divergence {
        Some((index, child_row, real_row)) => println!(
            "cursor rows first disagree at record {index}: child {child_row}, real {real_row}"
        ),
        None => println!("cursor rows never disagreed"),
    }
    match rows_diverged {
        Some((index, rows)) => {
            println!("row text first disagrees by record {index}: rows {rows:?}")
        }
        None => println!("row text never disagreed"),
    }

    println!("\n-- what the proxy meant to paint --");
    for (index, row) in intended(&child).iter().enumerate() {
        println!("{index:3} |{row}");
    }
    println!("\n-- what the terminal was actually painted --");
    for (index, row) in screen(&real).iter().enumerate() {
        println!("{index:3} |{row}");
    }
    Ok(())
}
