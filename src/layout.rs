use std::collections::BTreeMap;

use unicode_bidi::{BidiInfo, Level};
use unicode_bidi_mirroring::get_mirrored;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::classify::{select_mode, ExecutionPath, RowDisposition};
use crate::config::Mode;
use crate::terminal::{CellSnapshot, CellWidth, PaneSpan, PhysicalRowSnapshot};

const MAX_GRAPHEMES: usize = 2_000;
const PROMPTS: &[&str] = &["❯", ">", "»", "❱", "›"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoordinateMap {
    pub logical_start: usize,
    pub logical_end: usize,
    pub logical_to_visual: Vec<u16>,
    pub visual_to_logical: Vec<Option<usize>>,
}

impl CoordinateMap {
    pub fn visual_col(&self, logical_col: usize) -> Option<u16> {
        (self.logical_start..=self.logical_end)
            .contains(&logical_col)
            .then(|| self.logical_to_visual[logical_col - self.logical_start])
    }

    pub fn logical_col(&self, visual_col: u16) -> Option<usize> {
        self.visual_to_logical
            .get(usize::from(visual_col))
            .copied()
            .flatten()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutResult {
    pub cells: Vec<CellSnapshot>,
    pub transformed: bool,
    pub right_aligned: bool,
    pub logical_text: Option<String>,
    pub coordinates: Option<CoordinateMap>,
}

#[derive(Clone, Eq, PartialEq)]
struct Token {
    cell: CellSnapshot,
    logical_col: usize,
    rtl: bool,
}

#[derive(Clone)]
struct Resolution {
    tokens: Vec<Token>,
    levels: Vec<Level>,
    order: Vec<usize>,
    base_rtl: bool,
}

pub fn layout_row(
    row: &PhysicalRowSnapshot,
    pane: PaneSpan,
    path: &ExecutionPath,
    mode: Mode,
) -> LayoutResult {
    layout_rows(std::slice::from_ref(row), pane, path, mode)
        .into_iter()
        .next()
        .unwrap_or_else(|| unchanged(row))
}

pub fn layout_rows(
    rows: &[PhysicalRowSnapshot],
    pane: PaneSpan,
    path: &ExecutionPath,
    mode: Mode,
) -> Vec<LayoutResult> {
    if rows.is_empty() {
        return Vec::new();
    }
    let selection = select_mode(mode, path);
    if selection.disposition == RowDisposition::PassThrough || pane.start_col >= pane.end_col {
        return rows.iter().map(unchanged).collect();
    }

    let mut output = Vec::with_capacity(rows.len());
    let mut start = 0;
    while start < rows.len() {
        let mut end = start;
        while end + 1 < rows.len() && rows[end].soft_wrapped {
            end += 1;
        }
        output.extend(layout_group(
            &rows[start..=end],
            pane,
            selection.disposition,
        ));
        start = end + 1;
    }
    output
}

fn layout_group(
    rows: &[PhysicalRowSnapshot],
    pane: PaneSpan,
    disposition: RowDisposition,
) -> Vec<LayoutResult> {
    let width = usize::from(pane.end_col - pane.start_col);
    let painted = rows
        .iter()
        .flat_map(|row| tokens_in(row, pane, row.soft_wrapped))
        .collect::<Vec<_>>();
    if painted.is_empty() || painted.len() > MAX_GRAPHEMES || !contains_rtl(&painted) {
        return rows.iter().map(unchanged).collect();
    }

    let logical = match disposition {
        RowDisposition::TransformLogical => painted,
        RowDisposition::RecoverVisual => match recover_visual(painted) {
            Some(tokens) => tokens,
            None => return rows.iter().map(unchanged).collect(),
        },
        RowDisposition::PassThrough => return rows.iter().map(unchanged).collect(),
    };
    let mut logical = logical;
    assign_logical_columns(&mut logical);
    let logical_text = text_of(&logical);
    let Some(wrapped) = wrap_tokens(logical, width) else {
        return rows.iter().map(unchanged).collect();
    };
    if wrapped.len() > rows.len() {
        return rows.iter().map(unchanged).collect();
    }

    let mut results = wrapped
        .into_iter()
        .enumerate()
        .map(|(index, tokens)| layout_logical_row(tokens, pane, &rows[index], &logical_text))
        .collect::<Vec<_>>();
    while results.len() < rows.len() {
        results.push(blank_result(&rows[results.len()], pane));
    }
    results
}

fn layout_logical_row(
    tokens: Vec<Token>,
    pane: PaneSpan,
    template: &PhysicalRowSnapshot,
    logical_text: &str,
) -> LayoutResult {
    let segments = split_layout_segments(tokens);
    let has_layout = segments.len() > 1;
    let mut visual = Vec::new();
    let mut base_rtl = false;
    for (segment, separator) in segments {
        if segment.is_empty() {
            if let Some(separator) = separator {
                visual.push(separator);
            }
            continue;
        }
        let prefix = prompt_prefix_len(&segment);
        visual.extend_from_slice(&segment[..prefix]);
        let content = segment[prefix..].to_vec();
        if let Some(resolution) = resolve(content) {
            base_rtl |= resolution.base_rtl;
            visual.extend(visual_tokens(&resolution, true));
        } else {
            visual.extend_from_slice(&segment[prefix..]);
        }
        if let Some(separator) = separator {
            visual.push(separator);
        }
    }

    let used = display_width(&visual);
    let pane_width = usize::from(pane.end_col - pane.start_col);
    let right_aligned = !has_layout && base_rtl && used < pane_width;
    let offset = if right_aligned { pane_width - used } else { 0 };
    let coordinates = coordinate_map(&visual, pane, offset);
    let cells = paint_into(template, pane, &visual, offset);
    LayoutResult {
        transformed: cells != template.cells,
        cells,
        right_aligned,
        logical_text: Some(logical_text.to_owned()),
        coordinates,
    }
}

fn recover_visual(painted: Vec<Token>) -> Option<Vec<Token>> {
    let mut logical = Vec::with_capacity(painted.len());
    for (segment, separator) in split_layout_segments(painted) {
        let prefix = prompt_prefix_len(&segment);
        logical.extend_from_slice(&segment[..prefix]);
        let content = &segment[prefix..];
        if contains_rtl(content) {
            logical.extend(recover(content)?);
        } else {
            logical.extend_from_slice(content);
        }
        if let Some(separator) = separator {
            logical.push(separator);
        }
    }
    Some(logical)
}

fn recover(painted: &[Token]) -> Option<Vec<Token>> {
    let mut guesses = vec![painted.to_vec()];
    let mut reversed = painted.to_vec();
    reversed.reverse();
    guesses.push(reversed);
    let mut found: Vec<Vec<Token>> = Vec::new();

    for mut candidate in guesses {
        for _ in 0..6 {
            let resolutions = resolutions_for_recovery(candidate.clone())?;
            if resolutions
                .iter()
                .any(|resolution| same_text(&visual_tokens(resolution, false), painted))
            {
                if !found.iter().any(|existing| same_text(existing, &candidate)) {
                    found.push(candidate);
                }
                break;
            }
            let resolution = &resolutions[0];
            if resolution.order.len() != painted.len() {
                break;
            }
            let mut next = candidate.clone();
            for (visual, &logical) in resolution.order.iter().enumerate() {
                next[logical] = painted[visual].clone();
            }
            if same_text(&next, &candidate) {
                break;
            }
            candidate = next;
        }
    }

    let first = found.first()?.clone();
    let first_visual = visual_tokens(&resolve(first.clone())?, true);
    if found.iter().skip(1).any(|candidate| {
        resolve(candidate.clone())
            .map(|resolution| visual_tokens(&resolution, true))
            .is_none_or(|visual| visual != first_visual)
    }) {
        None
    } else {
        Some(first)
    }
}

fn resolve(tokens: Vec<Token>) -> Option<Resolution> {
    let base = preferred_base(&tokens);
    resolve_with_base(tokens, base)
}

fn resolutions_for_recovery(tokens: Vec<Token>) -> Option<Vec<Resolution>> {
    let preferred = preferred_base(&tokens);
    let first = resolve_with_base(tokens.clone(), preferred)?;
    let mut resolutions = vec![first];
    if preferred.is_some() {
        resolutions.push(resolve_with_base(tokens, None)?);
    }
    Some(resolutions)
}

fn preferred_base(tokens: &[Token]) -> Option<Level> {
    let text = text_of(tokens);
    let rtl = text.chars().filter(|ch| is_rtl_char(*ch)).count();
    let ltr = text.chars().filter(|ch| ch.is_ascii_alphabetic()).count();
    (rtl > 0 && rtl >= ltr).then(Level::rtl)
}

fn resolve_with_base(tokens: Vec<Token>, base: Option<Level>) -> Option<Resolution> {
    if tokens.is_empty() {
        return None;
    }
    let text = text_of(&tokens);
    let info = BidiInfo::new(&text, base);
    let paragraph = info.paragraphs.first()?;
    let byte_levels = info.reordered_levels(paragraph, paragraph.range.clone());
    let levels = text
        .grapheme_indices(true)
        .map(|(offset, _)| byte_levels[offset])
        .collect::<Vec<_>>();
    if levels.len() != tokens.len() {
        return None;
    }
    let order = BidiInfo::reorder_visual(&levels);
    Some(Resolution {
        tokens,
        levels,
        order,
        base_rtl: paragraph.level.is_rtl(),
    })
}

fn visual_tokens(resolution: &Resolution, mirror: bool) -> Vec<Token> {
    resolution
        .order
        .iter()
        .map(|&logical| {
            let mut token = resolution.tokens[logical].clone();
            token.rtl = resolution.levels[logical].is_rtl();
            if mirror && token.rtl {
                token.cell.text = mirror_text(&token.cell.text);
            }
            token
        })
        .collect()
}

fn mirror_text(text: &str) -> String {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    match get_mirrored(first) {
        Some(mirrored) => std::iter::once(mirrored).chain(chars).collect(),
        None => text.to_owned(),
    }
}

fn wrap_tokens(tokens: Vec<Token>, width: usize) -> Option<Vec<Vec<Token>>> {
    if width == 0 {
        return None;
    }
    let mut rows = vec![Vec::new()];
    let mut used = 0;
    for token in tokens {
        let token_width = token_width(&token);
        if used > 0 && used + token_width > width {
            rows.push(Vec::new());
            used = 0;
        }
        if token_width > width {
            return None;
        }
        used += token_width;
        rows.last_mut().unwrap().push(token);
    }
    Some(rows)
}

fn tokens_in(row: &PhysicalRowSnapshot, pane: PaneSpan, keep_padding: bool) -> Vec<Token> {
    let start = usize::from(pane.start_col);
    let end = usize::from(pane.end_col).min(row.cells.len());
    let mut tokens = row.cells[start..end]
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.width != CellWidth::Continuation)
        .map(|(_, cell)| Token {
            cell: if cell.width == CellWidth::Empty {
                CellSnapshot {
                    text: " ".to_owned(),
                    style: cell.style,
                    width: CellWidth::Single,
                }
            } else {
                cell.clone()
            },
            logical_col: 0,
            rtl: false,
        })
        .collect::<Vec<_>>();
    if !keep_padding {
        while tokens.last().is_some_and(is_space) {
            tokens.pop();
        }
    }
    tokens
}

fn split_layout_segments(tokens: Vec<Token>) -> Vec<(Vec<Token>, Option<Token>)> {
    let mut result = Vec::new();
    let mut current = Vec::new();
    for token in tokens {
        if token.cell.text.chars().any(is_layout) {
            result.push((std::mem::take(&mut current), Some(token)));
        } else {
            current.push(token);
        }
    }
    result.push((current, None));
    result
}

fn paint_into(
    template: &PhysicalRowSnapshot,
    pane: PaneSpan,
    tokens: &[Token],
    offset: usize,
) -> Vec<CellSnapshot> {
    let mut cells = template.cells.clone();
    let start = usize::from(pane.start_col);
    let end = usize::from(pane.end_col).min(cells.len());
    for cell in &mut cells[start..end] {
        cell.text.clear();
        cell.width = CellWidth::Empty;
    }
    let mut col = start + offset;
    for token in tokens {
        let width = token_width(token);
        if col + width > end {
            break;
        }
        cells[col] = token.cell.clone();
        if width == 2 {
            cells[col].width = CellWidth::Wide;
            cells[col + 1] = CellSnapshot {
                text: String::new(),
                style: token.cell.style,
                width: CellWidth::Continuation,
            };
        } else {
            cells[col].width = CellWidth::Single;
        }
        col += width;
    }
    cells
}

fn prompt_prefix_len(tokens: &[Token]) -> usize {
    if tokens.len() >= 2 && PROMPTS.contains(&tokens[0].cell.text.as_str()) && is_space(&tokens[1])
    {
        2
    } else if tokens
        .first()
        .is_some_and(|token| PROMPTS.contains(&token.cell.text.as_str()))
    {
        1
    } else if tokens.len() >= 2 && is_space(&tokens[0]) && is_space(&tokens[1]) {
        2
    } else {
        0
    }
}

fn assign_logical_columns(tokens: &mut [Token]) {
    let mut col = 0;
    for token in tokens {
        token.logical_col = col;
        col += token_width(token);
    }
}

fn coordinate_map(tokens: &[Token], pane: PaneSpan, offset: usize) -> Option<CoordinateMap> {
    let logical_start = tokens.iter().map(|token| token.logical_col).min()?;
    let logical_end = tokens
        .iter()
        .map(|token| token.logical_col + token_width(token))
        .max()?;
    let mut positions = BTreeMap::new();
    let mut visual_col = usize::from(pane.start_col) + offset;
    for token in tokens {
        positions.insert(
            token.logical_col,
            (visual_col, token_width(token), token.rtl),
        );
        visual_col += token_width(token);
    }

    let mut logical_to_visual = vec![0; logical_end - logical_start + 1];
    let first = positions.get(&logical_start)?;
    logical_to_visual[0] = u16::try_from(if first.2 { first.0 + first.1 } else { first.0 }).ok()?;
    for (&logical_col, &(visual_col, width, rtl)) in &positions {
        for step in 1..=width {
            let mapped = if rtl {
                visual_col + width - step
            } else {
                visual_col + step
            };
            logical_to_visual[logical_col + step - logical_start] = u16::try_from(mapped).ok()?;
        }
    }

    let mut visual_to_logical = vec![None; usize::from(pane.end_col) + 1];
    for (offset, &visual) in logical_to_visual.iter().enumerate() {
        if let Some(slot) = visual_to_logical.get_mut(usize::from(visual)) {
            slot.get_or_insert(logical_start + offset);
        }
    }
    Some(CoordinateMap {
        logical_start,
        logical_end,
        logical_to_visual,
        visual_to_logical,
    })
}

fn display_width(tokens: &[Token]) -> usize {
    tokens.iter().map(token_width).sum()
}

fn token_width(token: &Token) -> usize {
    match token.cell.width {
        CellWidth::Wide => 2,
        CellWidth::Empty | CellWidth::Single | CellWidth::Continuation => {
            UnicodeWidthStr::width(token.cell.text.as_str()).max(1)
        }
    }
}

fn contains_rtl(tokens: &[Token]) -> bool {
    tokens
        .iter()
        .any(|token| token.cell.text.chars().any(is_rtl_char))
}

fn text_of(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|token| token.cell.text.as_str())
        .collect()
}

fn same_text(left: &[Token], right: &[Token]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.cell.text == right.cell.text)
}

fn is_space(token: &Token) -> bool {
    token.cell.text.chars().all(char::is_whitespace)
}

pub fn is_rtl_char(ch: char) -> bool {
    matches!(ch as u32, 0x0590..=0x08ff | 0xfb1d..=0xfdff | 0xfe70..=0xfeff)
}

fn is_layout(ch: char) -> bool {
    matches!(ch as u32, 0x2500..=0x259f | 0x2800..=0x28ff)
}

fn unchanged(row: &PhysicalRowSnapshot) -> LayoutResult {
    LayoutResult {
        cells: row.cells.clone(),
        transformed: false,
        right_aligned: false,
        logical_text: None,
        coordinates: None,
    }
}

fn blank_result(row: &PhysicalRowSnapshot, pane: PaneSpan) -> LayoutResult {
    let mut result = unchanged(row);
    let start = usize::from(pane.start_col).min(result.cells.len());
    let end = usize::from(pane.end_col).min(result.cells.len());
    for cell in &mut result.cells[start..end] {
        cell.text.clear();
        cell.width = CellWidth::Empty;
    }
    result.transformed = result.cells != row.cells;
    result
}
