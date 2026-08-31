use std::collections::BTreeMap;

use unicode_bidi::{BidiInfo, Level};
use unicode_bidi_mirroring::get_mirrored;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::classify::{select_mode, ExecutionPath, RowDisposition};
use crate::config::Mode;
use crate::terminal::{CellSnapshot, CellWidth, Color, PaneSpan, PhysicalRowSnapshot};

const MAX_GRAPHEMES: usize = 2_000;
const PROMPTS: &[&str] = &["❯", ">", "»", "❱", "›"];

#[derive(Clone, Copy)]
struct ParagraphPolicy {
    base: Level,
    align_right: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Prose,
    Boundary,
}

struct GroupLayout {
    results: Vec<LayoutResult>,
    policy: Option<ParagraphPolicy>,
}

struct LogicalRowLayout {
    result: LayoutResult,
    base_rtl: bool,
    resolved: bool,
}

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
    pub align_offset: u16,
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
    let mut active_paragraph = None;
    let mut start = 0;
    while start < rows.len() {
        let mut end = start;
        while end + 1 < rows.len() && rows[end].soft_wrapped {
            end += 1;
        }
        let group = &rows[start..=end];
        let kind = classify_group(group, pane);
        let inherited = (kind == RowKind::Prose)
            .then_some(active_paragraph)
            .flatten();
        let layout = layout_group(group, pane, selection.disposition, inherited);
        if kind == RowKind::Boundary {
            active_paragraph = None;
        } else if inherited.is_none() {
            active_paragraph = layout.policy;
        }
        output.extend(layout.results);
        start = end + 1;
    }
    output
}

fn layout_group(
    rows: &[PhysicalRowSnapshot],
    pane: PaneSpan,
    disposition: RowDisposition,
    inherited: Option<ParagraphPolicy>,
) -> GroupLayout {
    let fallback = || GroupLayout {
        results: rows.iter().map(unchanged).collect(),
        policy: None,
    };
    let width = usize::from(pane.end_col - pane.start_col);
    let painted = rows
        .iter()
        .flat_map(|row| tokens_in(row, pane, row.soft_wrapped))
        .collect::<Vec<_>>();
    if painted.is_empty() || painted.len() > MAX_GRAPHEMES {
        return fallback();
    }
    if !contains_rtl(&painted) && inherited.is_none() {
        return GroupLayout {
            results: rows.iter().map(unchanged).collect(),
            policy: Some(ParagraphPolicy {
                base: Level::ltr(),
                align_right: false,
            }),
        };
    }

    let logical = if !contains_rtl(&painted) {
        painted
    } else {
        match disposition {
            RowDisposition::TransformLogical => painted,
            RowDisposition::RecoverVisual => {
                match recover_visual(painted, inherited.map(|policy| policy.base)) {
                    Some(tokens) => tokens,
                    None => return fallback(),
                }
            }
            RowDisposition::PassThrough => return fallback(),
        }
    };
    let mut logical = logical;
    assign_logical_columns(&mut logical);
    let logical_text = text_of(&logical);
    let Some(wrapped) = wrap_tokens(logical, width) else {
        return fallback();
    };
    if wrapped.len() > rows.len() {
        return fallback();
    }

    let mut wrapped = wrapped.into_iter();
    let Some(first_tokens) = wrapped.next() else {
        return fallback();
    };
    let first = layout_logical_row(first_tokens, pane, &rows[0], &logical_text, inherited);
    if !first.resolved {
        return fallback();
    }
    let policy = inherited.or_else(|| {
        Some(ParagraphPolicy {
            base: if first.base_rtl {
                Level::rtl()
            } else {
                Level::ltr()
            },
            align_right: first.base_rtl,
        })
    });
    let mut results = vec![first.result];
    for (index, tokens) in wrapped.enumerate() {
        let row = layout_logical_row(tokens, pane, &rows[index + 1], &logical_text, policy);
        if !row.resolved {
            return fallback();
        }
        results.push(row.result);
    }
    while results.len() < rows.len() {
        results.push(blank_result(&rows[results.len()], pane));
    }
    GroupLayout { results, policy }
}

fn classify_group(rows: &[PhysicalRowSnapshot], pane: PaneSpan) -> RowKind {
    let first_text = pane_text(&rows[0], pane);
    let all_blank = rows
        .iter()
        .all(|row| pane_text(row, pane).trim().is_empty());
    if all_blank {
        return RowKind::Boundary;
    }

    let first_trimmed = first_text.trim_start();
    if is_indented_code(&first_text)
        || is_list_item(first_trimmed)
        || row_is_lexical_boundary(&rows[0], pane)
        || rows.iter().any(|row| row_is_intrinsic_boundary(row, pane))
    {
        RowKind::Boundary
    } else {
        RowKind::Prose
    }
}

fn row_is_lexical_boundary(row: &PhysicalRowSnapshot, pane: PaneSpan) -> bool {
    let text = pane_text(row, pane);
    let trimmed = text.trim();
    trimmed.is_empty()
        || trimmed.starts_with("```")
        || trimmed.starts_with("~~~")
        || is_standalone_url(trimmed)
        || is_prompt_row(trimmed)
        || is_table_or_layout_row(trimmed)
        || is_separator(trimmed)
}

fn row_is_intrinsic_boundary(row: &PhysicalRowSnapshot, pane: PaneSpan) -> bool {
    same_hyperlink_content(row, pane) || row_wide_style(row, pane)
}

fn is_indented_code(text: &str) -> bool {
    text.starts_with('\t')
        || (text.starts_with("    ") && !text.as_bytes().get(4).is_some_and(|byte| *byte == b' '))
        || (text.starts_with("        ")
            && !text.as_bytes().get(8).is_some_and(|byte| *byte == b' '))
}

fn pane_text(row: &PhysicalRowSnapshot, pane: PaneSpan) -> String {
    text_of(&tokens_in(row, pane, false))
}

fn is_list_item(text: &str) -> bool {
    let mut chars = text.chars();
    if matches!(chars.next(), Some('-' | '*' | '+' | '•' | '◦' | '▪')) {
        return chars.next().is_some_and(char::is_whitespace);
    }
    let mut digits = 0;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digits += 1;
        } else {
            break;
        }
    }
    if digits == 0 {
        return false;
    }
    let suffix = &text[digits..];
    let mut chars = suffix.chars();
    matches!(chars.next(), Some('.' | ')')) && chars.next().is_some_and(char::is_whitespace)
}

fn is_standalone_url(text: &str) -> bool {
    (text.starts_with("http://") || text.starts_with("https://"))
        && !text.chars().any(char::is_whitespace)
}

fn is_prompt_row(text: &str) -> bool {
    PROMPTS.iter().any(|prompt| {
        text.strip_prefix(prompt).is_some_and(|rest| {
            rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace)
        })
    })
}

fn is_table_or_layout_row(text: &str) -> bool {
    text.chars().any(is_layout) || text.matches('|').count() >= 2
}

fn is_separator(text: &str) -> bool {
    let visible = text.chars().filter(|ch| !ch.is_whitespace()).count();
    visible >= 3
        && text
            .chars()
            .all(|ch| ch.is_whitespace() || matches!(ch, '-' | '=' | '_' | '*' | '~' | ':' | '|'))
}

fn same_hyperlink_content(row: &PhysicalRowSnapshot, pane: PaneSpan) -> bool {
    let start = usize::from(pane.start_col);
    let end = usize::from(pane.end_col).min(row.cells.len());
    let mut uri = None;
    let mut content = false;
    for cell in &row.cells[start..end] {
        if cell.width == CellWidth::Continuation || cell.text.trim().is_empty() {
            continue;
        }
        content = true;
        let Some(cell_uri) = cell.hyperlink.as_deref() else {
            return false;
        };
        if uri.is_some_and(|known| known != cell_uri) {
            return false;
        }
        uri = Some(cell_uri);
    }
    content && uri.is_some()
}

fn row_wide_style(row: &PhysicalRowSnapshot, pane: PaneSpan) -> bool {
    let start = usize::from(pane.start_col);
    let end = usize::from(pane.end_col).min(row.cells.len());
    let Some(first) = row.cells[start..end]
        .iter()
        .position(|cell| !cell.text.trim().is_empty())
    else {
        return false;
    };
    let cells = &row.cells[start + first..end];
    let background = cells[0].style.background;
    (background != Color::Default && cells.iter().all(|cell| cell.style.background == background))
        || cells.iter().all(|cell| cell.style.inverse)
}

fn layout_logical_row(
    tokens: Vec<Token>,
    pane: PaneSpan,
    template: &PhysicalRowSnapshot,
    logical_text: &str,
    inherited: Option<ParagraphPolicy>,
) -> LogicalRowLayout {
    let segments = split_layout_segments(tokens);
    let has_layout = segments.len() > 1;
    let mut visual = Vec::new();
    let mut base_rtl = false;
    let mut resolved = true;
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
        let base = inherited
            .map(|policy| policy.base)
            .or_else(|| preferred_base(&content));
        if let Some(resolution) = resolve_with_base(content, base) {
            base_rtl |= resolution.base_rtl;
            visual.extend(visual_tokens(&resolution, true));
        } else {
            resolved = false;
            visual.extend_from_slice(&segment[prefix..]);
        }
        if let Some(separator) = separator {
            visual.push(separator);
        }
    }

    let used = display_width(&visual);
    let pane_width = usize::from(pane.end_col - pane.start_col);
    let right_aligned = if has_layout {
        false
    } else if let Some(policy) = inherited {
        policy.align_right
    } else {
        base_rtl && used < pane_width
    };
    let offset = if right_aligned && used < pane_width {
        pane_width - used
    } else {
        0
    };
    let coordinates = coordinate_map(&visual, pane, offset);
    let cells = paint_into(template, pane, &visual, offset);
    LogicalRowLayout {
        base_rtl,
        resolved,
        result: LayoutResult {
            transformed: cells != template.cells,
            cells,
            right_aligned,
            align_offset: u16::try_from(offset).unwrap_or(0),
            logical_text: Some(logical_text.to_owned()),
            coordinates,
        },
    }
}

fn recover_visual(painted: Vec<Token>, inherited_base: Option<Level>) -> Option<Vec<Token>> {
    let mut logical = Vec::with_capacity(painted.len());
    for (segment, separator) in split_layout_segments(painted) {
        let prefix = prompt_prefix_len(&segment);
        logical.extend_from_slice(&segment[..prefix]);
        let content = &segment[prefix..];
        if contains_rtl(content) {
            logical.extend(recover(content, inherited_base)?);
        } else {
            logical.extend_from_slice(content);
        }
        if let Some(separator) = separator {
            logical.push(separator);
        }
    }
    Some(logical)
}

fn recover(painted: &[Token], inherited_base: Option<Level>) -> Option<Vec<Token>> {
    let mut guesses = vec![painted.to_vec()];
    let mut reversed = painted.to_vec();
    reversed.reverse();
    guesses.push(reversed);
    let mut found: Vec<Vec<Token>> = Vec::new();

    for mut candidate in guesses {
        for _ in 0..6 {
            let resolutions = resolutions_for_recovery(candidate.clone(), inherited_base)?;
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
    let first_visual = visual_tokens(
        &resolve_with_base(
            first.clone(),
            inherited_base.or_else(|| preferred_base(&first)),
        )?,
        true,
    );
    if found.iter().skip(1).any(|candidate| {
        resolve_with_base(
            candidate.clone(),
            inherited_base.or_else(|| preferred_base(candidate)),
        )
        .map(|resolution| visual_tokens(&resolution, true))
        .is_none_or(|visual| visual != first_visual)
    }) {
        None
    } else {
        Some(first)
    }
}

fn resolutions_for_recovery(
    tokens: Vec<Token>,
    inherited_base: Option<Level>,
) -> Option<Vec<Resolution>> {
    let preferred = inherited_base.or_else(|| preferred_base(&tokens));
    let first = resolve_with_base(tokens.clone(), preferred)?;
    let mut resolutions = vec![first];
    if preferred.is_some() && inherited_base.is_none() {
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
                    hyperlink: None,
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
        cell.hyperlink = None;
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
                hyperlink: None,
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
        align_offset: 0,
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
        cell.hyperlink = None;
        cell.width = CellWidth::Empty;
    }
    result.transformed = result.cells != row.cells;
    result
}
