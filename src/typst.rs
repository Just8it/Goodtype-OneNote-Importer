//! Typst markup for OneNote text. Pure functions over this importer's own small model, so the
//! rules are testable without OneNote files.
//!
//! Every embedded expression ends in `;`, which Typst consumes: text that follows a function
//! (`(`, `[`, `.`) can then never be read as a call or field access. User text is escaped
//! character by character, so it can never become markup, code or a label.

/// One run of text with uniform formatting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub size_pt: Option<f64>,
    pub color: Option<[u8; 3]>,
    pub highlight: Option<[u8; 3]>,
    pub link: Option<String>,
    /// Typst math source; when set, `text` is only the equation's plain reading.
    pub math: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ListKind {
    Bullet,
    Numbered,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    /// Outline nesting depth, 0 at the top.
    pub level: usize,
    pub list: Option<ListKind>,
    /// Explicit number of a numbered item, so a list split across pages keeps counting.
    pub number: Option<usize>,
    pub align: Align,
    /// A OneNote to-do tag: `Some(done)`.
    pub checkbox: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Paragraph(Paragraph),
    Table {
        level: usize,
        widths_pt: Vec<f64>,
        borders: bool,
        /// Rows of cells; each cell holds its own items.
        rows: Vec<Vec<Vec<Item>>>,
    },
    /// A picture in the text flow. `path` is notebook-root absolute (`/assets/…`).
    Image {
        level: usize,
        path: String,
        width_pt: f64,
        height_pt: f64,
    },
    /// A short framed note, e.g. where an attachment sat.
    Note {
        level: usize,
        text: String,
    },
}

/// The body font size OneNote uses when a run names none.
pub const DEFAULT_SIZE_PT: f64 = 11.0;
const INDENT_PT: f64 = 18.0;

/// Backslash-escapes every character that could start markup, code, math, a label, a reference
/// or a comment where it stands, and nothing else, so imported text stays readable to edit.
/// Line breaks become Typst line breaks; other control characters are dropped.
pub fn escape(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    // At the start of a line, `-`, `+`, `=`, `/` and `12.` begin lists, headings and terms.
    let mut line_start = true;
    let mut digits_only = true;
    for (index, &c) in chars.iter().enumerate() {
        let next = chars.get(index + 1).copied();
        let escaped = match c {
            '\\' | '#' | '*' | '_' | '`' | '$' | '<' | '>' | '@' | '[' | ']' | '~' => true,
            '/' => line_start || matches!(next, Some('/' | '*')),
            '-' => line_start || matches!(next, Some('-' | '?')),
            '+' | '=' => line_start,
            '.' => !line_start && digits_only,
            _ => false,
        };
        match c {
            '\n' | '\r' | '\u{b}' | '\u{2028}' => {
                out.push_str("\\ ");
                line_start = true;
                digits_only = true;
                continue;
            }
            '\t' => out.push(' '),
            c if c.is_control() => {}
            c => {
                if escaped {
                    out.push('\\');
                }
                out.push(c);
            }
        }
        if !c.is_whitespace() {
            digits_only = line_start && c.is_ascii_digit()
                || !line_start && digits_only && c.is_ascii_digit();
            line_start = false;
        }
    }
    out
}

/// A Typst string literal.
pub fn string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn color([r, g, b]: [u8; 3]) -> String {
    format!("rgb(\"#{r:02x}{g:02x}{b:02x}\")")
}

fn number(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

/// One run as markup, wrapped innermost-first so styles compose.
pub fn run(run: &Run) -> String {
    if let Some(math) = &run.math {
        // No space inside the dollars: inline, not a display equation.
        return format!("${}$", math.trim());
    }
    let mut body = escape(&run.text);
    if body.trim().is_empty() {
        return body;
    }
    let wrap = |name: &str, body: String| format!("#{name}[{body}];");
    if run.superscript {
        body = wrap("super", body);
    }
    if run.subscript {
        body = wrap("sub", body);
    }
    if run.strike {
        body = wrap("strike", body);
    }
    if run.underline && run.link.is_none() {
        body = wrap("underline", body);
    }
    if run.italic {
        body = wrap("emph", body);
    }
    if run.bold {
        body = wrap("strong", body);
    }
    let mut text_args = Vec::new();
    if let Some(size) = run
        .size_pt
        .filter(|size| (size - DEFAULT_SIZE_PT).abs() > 0.01)
    {
        text_args.push(format!("size: {}pt", number(size)));
    }
    if let Some(fill) = run.color.filter(|fill| *fill != [0, 0, 0]) {
        text_args.push(format!("fill: {}", color(fill)));
    }
    if !text_args.is_empty() {
        body = format!("#text({})[{body}];", text_args.join(", "));
    }
    if let Some(fill) = run.highlight {
        body = format!("#highlight(fill: {})[{body}];", color(fill));
    }
    if let Some(target) = &run.link {
        body = format!("#link({})[{body}];", string(target));
    }
    body
}

fn checkbox(done: bool) -> &'static str {
    if done {
        "#box(width: 0.8em, height: 0.8em, stroke: 0.07em, baseline: 0.12em)[#align(center + horizon)[#text(size: 0.75em)[✓]]]; "
    } else {
        "#box(width: 0.8em, height: 0.8em, stroke: 0.07em, baseline: 0.12em); "
    }
}

/// The paragraph's text, without list marker or indentation.
fn paragraph_text(paragraph: &Paragraph) -> String {
    let mut text: String = paragraph.runs.iter().map(run).collect();
    text = text.trim_start().to_owned();
    if let Some(done) = paragraph.checkbox {
        text.insert_str(0, checkbox(done));
    }
    text
}

fn paragraph(paragraph: &Paragraph) -> String {
    let text = paragraph_text(paragraph);
    if let Some(kind) = paragraph.list {
        let marker = match (kind, paragraph.number) {
            (ListKind::Bullet, _) => "-".to_owned(),
            (ListKind::Numbered, Some(number)) => format!("{number}."),
            (ListKind::Numbered, None) => "+".to_owned(),
        };
        return format!("{}{marker} {text}", "  ".repeat(paragraph.level));
    }
    let mut body = if text.is_empty() {
        // An empty OneNote paragraph is a deliberate blank line.
        "#v(1em);".to_owned()
    } else {
        text
    };
    match paragraph.align {
        Align::Left => {}
        Align::Center => body = format!("#align(center)[{body}];"),
        Align::Right => body = format!("#align(right)[{body}];"),
    }
    indent(paragraph.level, body)
}

fn indent(level: usize, body: String) -> String {
    if level == 0 {
        body
    } else {
        format!(
            "#pad(left: {}pt)[{body}];",
            number(level as f64 * INDENT_PT)
        )
    }
}

fn item(item: &Item) -> String {
    match item {
        Item::Paragraph(p) => paragraph(p),
        Item::Table {
            level,
            widths_pt,
            borders,
            rows,
        } => {
            let columns = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
            let widths: Vec<String> = (0..columns)
                .map(|index| match widths_pt.get(index) {
                    Some(width) if *width > 0.0 => format!("{}pt", number(*width)),
                    _ => "auto".to_owned(),
                })
                .collect();
            let mut cells = Vec::new();
            for row in rows {
                for index in 0..columns {
                    let content = row.get(index).map(|cell| items(cell)).unwrap_or_default();
                    cells.push(format!("[{content}]"));
                }
            }
            let stroke = if *borders { "0.5pt" } else { "none" };
            indent(
                *level,
                format!(
                    "#table(columns: ({},), stroke: {stroke}, inset: 4pt, {});",
                    widths.join(", "),
                    cells.join(", ")
                ),
            )
        }
        Item::Image {
            level,
            path,
            width_pt,
            ..
        } => indent(
            *level,
            format!("#image({}, width: {}pt);", string(path), number(*width_pt)),
        ),
        Item::Note { level, text } => indent(
            *level,
            format!(
                "#box(stroke: 0.5pt, inset: 4pt, radius: 2pt)[{}];",
                escape(text)
            ),
        ),
    }
}

/// Items joined so consecutive list entries form one tight list and everything else stays a
/// separate paragraph.
pub fn items(items: &[Item]) -> String {
    let mut out = String::new();
    let mut previous_list = false;
    for (index, entry) in items.iter().enumerate() {
        let is_list = matches!(entry, Item::Paragraph(p) if p.list.is_some());
        if index > 0 {
            out.push_str(if is_list && previous_list {
                "\n"
            } else {
                "\n\n"
            });
        }
        out.push_str(&item(entry));
        previous_list = is_list;
    }
    out
}

/// A complete block source. OneNote paragraphs sit as close as lines, so paragraph spacing
/// matches the line leading; a title starts from the title size.
pub fn block(items_source: &str, base_size_pt: Option<f64>) -> String {
    let mut source = String::from("#set par(spacing: 0.65em)\n");
    if let Some(size) = base_size_pt {
        source.push_str(&format!("#set text(size: {}pt)\n", number(size)));
    }
    source.push_str(items_source);
    source.push('\n');
    source
}

/// Numbers every numbered list item in order, per nesting level, the way Typst would count them
/// in one list. A bullet or plain paragraph at a level ends the count there and deeper.
pub fn number_lists(items: &mut [Item]) {
    let mut counters: Vec<usize> = Vec::new();
    for entry in items {
        let Item::Paragraph(paragraph) = entry else {
            continue;
        };
        let level = paragraph.level;
        match paragraph.list {
            Some(ListKind::Numbered) => {
                counters.resize(level + 1, 0);
                counters[level] += 1;
                paragraph.number = Some(counters[level]);
            }
            Some(ListKind::Bullet) => {
                counters.resize(level + 1, 0);
                counters[level] = 0;
            }
            None => counters.truncate(level),
        }
    }
}

/// Rough height of rendered items, for placing what follows inside the same OneNote container.
/// Goodtype measures every block itself when it renders it; this only seeds the stored size.
pub fn estimate_height(items: &[Item], width_pt: f64, base_size_pt: f64) -> f64 {
    items
        .iter()
        .map(|entry| estimate_item(entry, width_pt, base_size_pt))
        .sum()
}

pub fn estimate_item(entry: &Item, width_pt: f64, base_size_pt: f64) -> f64 {
    {
        match entry {
            Item::Paragraph(p) => {
                let size = p
                    .runs
                    .iter()
                    .filter_map(|run| run.size_pt)
                    .fold(base_size_pt, f64::max);
                let chars: usize = p.runs.iter().map(|run| run.text.chars().count()).sum();
                let breaks: usize = p
                    .runs
                    .iter()
                    .map(|run| run.text.matches(['\n', '\u{b}']).count())
                    .sum();
                let usable = (width_pt - p.level as f64 * INDENT_PT).max(36.0);
                let per_line = (usable / (0.5 * size)).max(1.0);
                let lines = ((chars as f64 / per_line).ceil()).max(1.0) + breaks as f64;
                lines * 1.35 * size
            }
            Item::Table { rows, .. } => rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| estimate_height(cell, width_pt / 2.0, base_size_pt))
                        .fold(base_size_pt * 1.35, f64::max)
                        + 8.0
                })
                .sum(),
            Item::Image { height_pt, .. } => height_pt + 0.65 * base_size_pt,
            Item::Note { .. } => base_size_pt * 1.35 + 8.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Run {
        Run {
            text: value.into(),
            ..Run::default()
        }
    }

    #[test]
    fn user_text_never_becomes_markup_or_code() {
        let hostile = "#import \"x\": * $x$ <l> @r [a] // c /* d */ `raw` ~ \\";
        let escaped = escape(hostile);
        for (index, c) in escaped.char_indices() {
            if "#$<>@[]`*~".contains(c) {
                assert_eq!(
                    &escaped[index - 1..index],
                    "\\",
                    "{c} unescaped in {escaped}"
                );
            }
        }
        assert!(escaped.contains("\\// c \\/\\* d \\*/"));
        // Line starts: headings, lists, terms and numbered items stay text.
        assert_eq!(escape("= H"), "\\= H");
        assert_eq!(
            escape("  - a\n+ b\u{b}/ t: d"),
            "  \\- a\\ \\+ b\\ \\/ t: d"
        );
        assert_eq!(escape("12. Step"), "12\\. Step");
        // Ordinary prose is left alone.
        assert_eq!(
            escape("e.g. x-y = 3 + 4, it's \"ok\" (1. not a list)"),
            "e.g. x-y = 3 + 4, it's \"ok\" (1. not a list)"
        );
        assert_eq!(escape("a--b c-?"), "a\\--b c\\-?");
        assert_eq!(escape("a\u{b}b\tc\u{7}"), "a\\ b c");
    }

    #[test]
    fn runs_compose_styles_and_terminate_every_call() {
        let styled = Run {
            text: "f(x).y".into(),
            bold: true,
            italic: true,
            size_pt: Some(14.0),
            color: Some([255, 0, 0]),
            link: Some("https://example.com/\"q\"".into()),
            ..Run::default()
        };
        assert_eq!(
            run(&styled),
            "#link(\"https://example.com/\\\"q\\\"\")[#text(size: 14pt, fill: rgb(\"#ff0000\"))[#strong[#emph[f(x).y];];];];"
        );
        // Default size and black stay implicit.
        let plain = Run {
            size_pt: Some(11.0),
            color: Some([0, 0, 0]),
            ..text("plain")
        };
        assert_eq!(run(&plain), "plain");
    }

    #[test]
    fn lists_stay_tight_and_nest_by_level() {
        let list = |level, kind, value: &str| {
            Item::Paragraph(Paragraph {
                runs: vec![text(value)],
                level,
                list: Some(kind),
                ..Paragraph::default()
            })
        };
        let body = items(&[
            Item::Paragraph(Paragraph {
                runs: vec![text("Intro")],
                ..Paragraph::default()
            }),
            list(0, ListKind::Bullet, "one"),
            list(1, ListKind::Numbered, "two"),
            Item::Paragraph(Paragraph {
                runs: vec![text("Indented")],
                level: 1,
                align: Align::Center,
                ..Paragraph::default()
            }),
        ]);
        assert_eq!(
            body,
            "Intro\n\n- one\n  + two\n\n#pad(left: 18pt)[#align(center)[Indented];];"
        );
    }

    #[test]
    fn tables_images_notes_and_checkboxes() {
        let cell = |value: &str| {
            vec![Item::Paragraph(Paragraph {
                runs: vec![text(value)],
                ..Paragraph::default()
            })]
        };
        let table = Item::Table {
            level: 0,
            widths_pt: vec![72.0],
            borders: true,
            rows: vec![vec![cell("a"), cell("b")], vec![cell("c")]],
        };
        assert_eq!(
            items(&[table]),
            "#table(columns: (72pt, auto,), stroke: 0.5pt, inset: 4pt, [a], [b], [c], []);"
        );
        let image = Item::Image {
            level: 0,
            path: "/assets/p001-img001.png".into(),
            width_pt: 120.5,
            height_pt: 60.0,
        };
        assert_eq!(
            items(&[image]),
            "#image(\"/assets/p001-img001.png\", width: 120.5pt);"
        );
        let todo = Item::Paragraph(Paragraph {
            runs: vec![text("Buy milk")],
            checkbox: Some(false),
            ..Paragraph::default()
        });
        assert!(items(&[todo]).ends_with("baseline: 0.12em); Buy milk"));
        assert_eq!(
            block("Hi", Some(20.0)),
            "#set par(spacing: 0.65em)\n#set text(size: 20pt)\nHi\n"
        );
    }

    #[test]
    fn height_estimate_grows_with_text_and_wrapping() {
        let paragraph = |chars| {
            Item::Paragraph(Paragraph {
                runs: vec![text(&"x".repeat(chars))],
                ..Paragraph::default()
            })
        };
        let one_line = estimate_height(&[paragraph(10)], 300.0, 11.0);
        let wrapped = estimate_height(&[paragraph(400)], 300.0, 11.0);
        assert!((one_line - 14.85).abs() < 0.01);
        assert!(wrapped > 6.0 * one_line);
    }
}
