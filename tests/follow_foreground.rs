//! The relay under a shell that brings programs to the foreground one after
//! another: an unclassified program is forwarded byte for byte while the
//! screen model keeps following it, and a verdict that lands later repairs
//! only the rows that program painted.

use hebrew_tty::classify::{Confidence, ExecutionPath, Order, Wrapping};
use hebrew_tty::config::Mode;
use hebrew_tty::relay::Transform;

fn verified_visual() -> ExecutionPath {
    ExecutionPath {
        order: Some(Order::Visual),
        wrapping: Some(Wrapping::PostBidi),
        confidence: Confidence::Verified,
        evidence: Vec::new(),
    }
}

fn unknown() -> ExecutionPath {
    ExecutionPath {
        order: None,
        wrapping: None,
        confidence: Confidence::Unknown,
        evidence: Vec::new(),
    }
}

fn has_hebrew(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes)
        .chars()
        .any(|c| ('\u{5d0}'..='\u{5ea}').contains(&c))
}

#[test]
fn an_unclassified_program_is_forwarded_byte_for_byte_and_still_modelled() {
    let mut transform = Transform::new(Vec::new(), 4, 20, unknown(), Mode::Auto).unwrap();
    let bytes = "\x1b[1;1Hשלום עולם\r\nabc \x1b[31mdef\x1b[0m".as_bytes();
    transform.feed(bytes).unwrap();
    assert_eq!(transform.writer_mut().as_slice(), bytes);
    let screen = transform.model().snapshot();
    assert!(screen.physical_rows[0]
        .cells
        .iter()
        .any(|cell| cell.text == "ש"));
    assert_eq!(screen.cursor.row, 1);
}

#[test]
fn a_verdict_repairs_once_the_program_has_painted_rtl() {
    let mut transform = Transform::new(Vec::new(), 5, 20, unknown(), Mode::Auto).unwrap();
    transform.feed("\x1b[1;1Hold שורה".as_bytes()).unwrap();
    transform.mark_generation();
    transform.feed("\x1b[3;1Hשלום עולם".as_bytes()).unwrap();
    let before = transform.writer_mut().len();

    transform.set_path(verified_visual(), Mode::Auto).unwrap();

    let painted = transform.writer_mut()[before..].to_vec();
    assert!(has_hebrew(&painted), "the program's row is repaired: {painted:?}");
    assert!(
        String::from_utf8_lossy(&painted).contains("עולם"),
        "{painted:?}"
    );
}

#[test]
fn a_verdict_for_rows_painted_before_the_change_paints_nothing() {
    let mut transform = Transform::new(Vec::new(), 4, 20, unknown(), Mode::Auto).unwrap();
    transform.feed("\x1b[1;1Hשלום עולם".as_bytes()).unwrap();
    transform.mark_generation();
    let before = transform.writer_mut().len();

    transform.set_path(verified_visual(), Mode::Auto).unwrap();

    assert_eq!(transform.writer_mut().len(), before);
}

#[test]
fn leaving_an_agent_stops_repairs_and_hands_the_cursor_back_once() {
    let mut transform =
        Transform::new(Vec::new(), 4, 20, verified_visual(), Mode::Auto).unwrap();
    transform.feed("\x1b[1;1Hשלום עולם".as_bytes()).unwrap();
    assert!(transform.writer_mut().len() > "\x1b[1;1Hשלום עולם".len());

    transform.set_path(unknown(), Mode::Auto).unwrap();
    let before = transform.writer_mut().len();
    let prompt = b"\r\n$ ".as_slice();
    transform.feed(prompt).unwrap();
    let first = transform.writer_mut()[before..].to_vec();
    assert!(first.ends_with(prompt));
    assert!(first.starts_with(b"\x1b["), "the caret goes back where the agent left it");

    let before = transform.writer_mut().len();
    transform.feed(b"ls").unwrap();
    assert_eq!(&transform.writer_mut()[before..], b"ls");
}

#[test]
fn the_same_verdict_twice_is_silent() {
    let mut transform =
        Transform::new(Vec::new(), 4, 20, verified_visual(), Mode::Auto).unwrap();
    transform.feed("\x1b[1;1Hשלום".as_bytes()).unwrap();
    let before = transform.writer_mut().len();
    transform.set_path(verified_visual(), Mode::Auto).unwrap();
    assert_eq!(transform.writer_mut().len(), before);
}
