#![forbid(unsafe_code)]

//! Replays a `HEBREW_TTY_TRACE` recording: the child's own bytes into one
//! model, the bytes the real terminal received into another, and reports the
//! first record where the two screens stop agreeing.

use std::error::Error;

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
    let mut child = TerminalModel::new(rows, cols)?;
    let mut real = TerminalModel::new(rows, cols)?;
    let mut divergence = None;

    for (index, record) in records.iter().enumerate() {
        match record {
            TraceRecord::Input(bytes) => child.feed(bytes),
            TraceRecord::Output(bytes) => real.feed(bytes),
            TraceRecord::Resize { rows, cols } => {
                child.resize(*rows, *cols)?;
                real.resize(*rows, *cols)?;
            }
        }
        if divergence.is_none() {
            let (left, right) = (child.cursor(), real.cursor());
            if left.row != right.row {
                divergence = Some((index, left.row, right.row));
            }
        }
    }

    println!("records: {}", records.len());
    match divergence {
        Some((index, child_row, real_row)) => println!(
            "cursor rows first disagree at record {index}: child {child_row}, real {real_row}"
        ),
        None => println!("cursor rows never disagreed"),
    }

    println!("\n-- child's own screen --");
    for (index, row) in screen(&child).iter().enumerate() {
        println!("{index:3} |{row}");
    }
    println!("\n-- what the terminal was actually painted --");
    for (index, row) in screen(&real).iter().enumerate() {
        println!("{index:3} |{row}");
    }
    Ok(())
}
