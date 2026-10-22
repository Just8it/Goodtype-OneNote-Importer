//! OneNote equations as Typst math.
//!
//! OneNote keeps an equation as text in which U+FDD0 opens a math object, U+FDEE separates its
//! arguments and U+FDEF closes it; the paragraph lists the objects (fraction, superscript, …)
//! in the order they open. This builds that tree and writes it with Typst math functions.
//! Every character from the file is escaped or spaced so it can only ever be a symbol; anything
//! that does not parse cleanly comes back as `None`, and the caller keeps it as plain text.

const OPEN: char = '\u{fdd0}';
const SEPARATOR: char = '\u{fdee}';
const CLOSE: char = '\u{fdef}';
const MAX_DEPTH: usize = 32;

/// The object kinds this importer distinguishes; the rest render their arguments in sequence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Accent,
    Brackets,
    BracketsWithSeps,
    EquationArray,
    Fraction,
    /// `a/b` on one line.
    SlashedFraction,
    FunctionApply,
    LeftSubSup,
    LowerLimit,
    Matrix,
    Nary,
    Overbar,
    Radical,
    Stack,
    Subscript,
    SubSup,
    Superscript,
    Underbar,
    UpperLimit,
    /// Boxes, phantoms, plain text and kinds without their own rendering.
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Object {
    pub kind: Kind,
    pub column: Option<u8>,
    pub char: Option<char>,
    pub char1: Option<char>,
    pub char2: Option<char>,
}

#[derive(Debug)]
enum Node {
    Char(char),
    Object(Object, Vec<Vec<Node>>),
}

/// True when `text` holds OneNote math structure characters.
pub fn has_structure(text: &str) -> bool {
    text.contains([OPEN, SEPARATOR, CLOSE])
}

/// The text without math structure characters, for the plain-text fallback.
pub fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| ![OPEN, SEPARATOR, CLOSE].contains(c))
        .collect()
}

/// Typst math source (without `$`) for one equation, consuming its objects in order.
pub fn to_typst(text: &str, objects: &mut dyn Iterator<Item = Object>) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut position = 0;
    let nodes = sequence(&chars, &mut position, objects, 0)?;
    if position != chars.len() {
        return None;
    }
    let source = render(&nodes);
    (!source.trim().is_empty()).then_some(source)
}

fn sequence(
    chars: &[char],
    position: &mut usize,
    objects: &mut dyn Iterator<Item = Object>,
    depth: usize,
) -> Option<Vec<Node>> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut nodes = Vec::new();
    while let Some(&c) = chars.get(*position) {
        match c {
            SEPARATOR | CLOSE => break,
            OPEN => {
                *position += 1;
                let object = objects.next()?;
                let mut arguments = Vec::new();
                loop {
                    arguments.push(sequence(chars, position, objects, depth + 1)?);
                    match chars.get(*position) {
                        Some(&SEPARATOR) => *position += 1,
                        Some(&CLOSE) => {
                            *position += 1;
                            break;
                        }
                        _ => return None,
                    }
                }
                nodes.push(Node::Object(object, arguments));
            }
            c => {
                *position += 1;
                nodes.push(Node::Char(c));
            }
        }
    }
    Some(nodes)
}

/// Mathematical alphanumeric letters (𝑎, 𝐶, 𝛾) back to their base letters; Typst italicises
/// single letters itself, and plain letters work with every math font.
fn base_letter(c: char) -> char {
    let code = c as u32;
    let latin = |offset: u32| {
        let index = (code - offset) % 52;
        char::from_u32(if index < 26 {
            'A' as u32 + index
        } else {
            'a' as u32 + index - 26
        })
    };
    let greek = |offset: u32| {
        let index = (code - offset) % 58;
        // Each Greek block: 25 capitals (with ϴ), ∇, 25 small letters, ∂ and six variants.
        match index {
            0..=16 => char::from_u32('Α' as u32 + index),
            17 => Some('ϴ'),
            18..=24 => char::from_u32('Σ' as u32 + index - 18),
            26..=50 => char::from_u32('α' as u32 + index - 26),
            _ => None,
        }
    };
    let mapped = match code {
        0x1d400..=0x1d6a3 => latin(0x1d400),
        0x1d6a8..=0x1d7c9 => greek(0x1d6a8),
        _ => None,
    };
    mapped.unwrap_or(c)
}

/// One character as a math atom: letters stand alone (so neighbours never form a variable
/// name), digits stay together, and every ASCII punctuation mark is escaped so no shorthand,
/// attachment, fraction, alignment or argument separator can arise from file content.
fn atom(c: char, out: &mut String) {
    let c = base_letter(c);
    if c.is_alphabetic() {
        out.push(' ');
        out.push(c);
        out.push(' ');
    } else if c.is_ascii_digit() {
        out.push(c);
    } else if c.is_ascii_punctuation() {
        out.push('\\');
        out.push(c);
    } else if c.is_whitespace() {
        out.push(' ');
    } else if !(c.is_control()
        || ('\u{fdd0}'..='\u{fdef}').contains(&c)
        || matches!(c, '\u{2061}'..='\u{2064}' | '\u{200b}'..='\u{200f}'))
    {
        // Control characters, invisible operators and stray structure characters are dropped.
        out.push(' ');
        out.push(c);
        out.push(' ');
    }
}

/// A delimiter character for `lr`, escaped when ASCII.
fn delimiter(c: Option<char>, fallback: char) -> String {
    match c.unwrap_or(fallback) {
        '\0' => String::new(),
        c if c.is_ascii_punctuation() => format!("\\{c}"),
        c => c.to_string(),
    }
}

fn render(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            Node::Char(c) => atom(*c, &mut out),
            Node::Object(object, arguments) => out.push_str(&object_source(object, arguments)),
        }
    }
    out
}

/// An argument wrapped so it is exactly one function argument, even when empty.
fn argument(nodes: &[Node]) -> String {
    let source = render(nodes);
    if source.trim().is_empty() {
        "#none".to_owned()
    } else {
        source.trim().to_owned()
    }
}

fn object_source(object: &Object, arguments: &[Vec<Node>]) -> String {
    let arg = |index: usize| {
        arguments
            .get(index)
            .map_or_else(|| "#none".to_owned(), |nodes| argument(nodes))
    };
    let all = || {
        arguments
            .iter()
            .map(|nodes| render(nodes))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let count = arguments.len();
    let source = match object.kind {
        Kind::Fraction if count == 2 => format!("frac({}, {})", arg(0), arg(1)),
        Kind::SlashedFraction if count == 2 => {
            format!("{} \\/ {}", render(&arguments[0]), render(&arguments[1]))
        }
        Kind::Stack if count == 2 => format!("mat(delim: #none, {}; {})", arg(0), arg(1)),
        Kind::Superscript if count == 2 => format!("attach({}, t: {})", arg(0), arg(1)),
        Kind::Subscript if count == 2 => format!("attach({}, b: {})", arg(0), arg(1)),
        Kind::SubSup if count == 3 => {
            format!("attach({}, b: {}, t: {})", arg(0), arg(1), arg(2))
        }
        Kind::LeftSubSup if count == 3 => {
            format!("attach({}, bl: {}, tl: {})", arg(2), arg(0), arg(1))
        }
        Kind::LowerLimit if count == 2 => {
            format!("attach(limits({}), b: {})", name(&arguments[0]), arg(1))
        }
        Kind::UpperLimit if count == 2 => {
            format!("attach(limits({}), t: {})", name(&arguments[0]), arg(1))
        }
        Kind::Radical if count == 1 => format!("sqrt({})", arg(0)),
        Kind::Radical if count == 2 => {
            if arguments[0].is_empty() {
                format!("sqrt({})", arg(1))
            } else {
                format!("root({}, {})", arg(0), arg(1))
            }
        }
        Kind::Nary if count == 3 => {
            let mut operator = String::new();
            atom(object.char.unwrap_or('∫'), &mut operator);
            let operator = operator.trim().to_owned();
            let limits = match (arguments[0].is_empty(), arguments[1].is_empty()) {
                (true, true) => operator,
                (false, true) => format!("attach({operator}, b: {})", arg(0)),
                (true, false) => format!("attach({operator}, t: {})", arg(1)),
                (false, false) => format!("attach({operator}, b: {}, t: {})", arg(0), arg(1)),
            };
            format!("{limits} {}", render(&arguments[2]))
        }
        Kind::Brackets | Kind::BracketsWithSeps => {
            let separator = delimiter(object.char2, '|');
            let inner = arguments
                .iter()
                .map(|nodes| render(nodes))
                .collect::<Vec<_>>()
                .join(&format!(" {separator} "));
            let open = delimiter(object.char, '(');
            let close = delimiter(object.char1, ')');
            format!("lr({open} {inner} {close})")
        }
        Kind::FunctionApply if count == 2 => {
            format!("{} {}", name(&arguments[0]), render(&arguments[1]))
        }
        Kind::Accent if count == 1 => match object.char {
            Some(mark) if !mark.is_ascii() && !mark.is_control() => {
                format!("accent({}, {mark})", arg(0))
            }
            _ => render(&arguments[0]),
        },
        Kind::Overbar if count == 1 => format!("overline({})", arg(0)),
        Kind::Underbar if count == 1 => format!("underline({})", arg(0)),
        Kind::Matrix => {
            let columns = usize::from(object.column.unwrap_or(1).max(1));
            if !count.is_multiple_of(columns) {
                return all();
            }
            let rows: Vec<String> = arguments
                .chunks(columns)
                .map(|row| {
                    row.iter()
                        .map(|cell| argument(cell))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .collect();
            // OneNote brackets are their own object around the matrix.
            format!("mat(delim: #none, {})", rows.join("; "))
        }
        Kind::EquationArray => arguments
            .iter()
            .map(|nodes| render(nodes))
            .collect::<Vec<_>>()
            .join(" \\ "),
        _ => all(),
    };
    format!(" {source} ")
}

/// A function name such as `cos` or `lim` set upright; anything else as an ordinary argument.
fn name(nodes: &[Node]) -> String {
    match plain_letters(nodes) {
        Some(name) if name.len() > 1 => format!("op(\"{name}\")"),
        _ => argument(nodes),
    }
}

/// The letters of a function name, when the argument is plain ASCII letters only.
fn plain_letters(nodes: &[Node]) -> Option<String> {
    nodes
        .iter()
        .map(|node| match node {
            Node::Char(c) => Some(base_letter(*c)).filter(|c| c.is_ascii_alphabetic()),
            Node::Object(..) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(kind: Kind) -> Object {
        Object {
            kind,
            column: None,
            char: None,
            char1: None,
            char2: None,
        }
    }

    fn convert(text: &str, objects: Vec<Object>) -> Option<String> {
        to_typst(text, &mut objects.into_iter())
            .map(|source| source.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    #[test]
    fn pythagoras_and_a_fraction_inside_an_integral() {
        assert_eq!(
            convert(
                "\u{fdd0}𝑎\u{fdee}2\u{fdef}+\u{fdd0}𝑏\u{fdee}2\u{fdef}=\u{fdd0}𝑐\u{fdee}2\u{fdef}",
                vec![object(Kind::Superscript); 3]
            )
            .unwrap(),
            "attach(a, t: 2) \\+ attach(b, t: 2) \\= attach(c, t: 2)"
        );
        // ∮_γ f(z)/(z−x) dz
        let nary = Object {
            char: Some('∮'),
            ..object(Kind::Nary)
        };
        assert_eq!(
            convert(
                "\u{fdd0}𝛾\u{fdee}\u{fdee}\u{fdd0}𝑓(𝑧)\u{fdee}𝑧−𝑥\u{fdef}𝑑𝑧\u{fdef}",
                vec![nary, object(Kind::Fraction)]
            )
            .unwrap(),
            "attach(∮, b: γ) frac(f \\( z \\), z − x) d z"
        );
    }

    #[test]
    fn functions_roots_brackets_and_matrices() {
        let brackets = Object {
            char: Some('['),
            char1: Some(']'),
            ..object(Kind::Brackets)
        };
        assert_eq!(
            convert(
                "\u{fdd0}cos\u{fdee}\u{fdd0}𝑥\u{fdef}\u{fdef}",
                vec![object(Kind::FunctionApply), brackets]
            )
            .unwrap(),
            "op(\"cos\") lr(\\[ x \\])"
        );
        assert_eq!(
            convert("\u{fdd0}3\u{fdee}𝑥\u{fdef}", vec![object(Kind::Radical)]).unwrap(),
            "root(3, x)"
        );
        let matrix = Object {
            column: Some(2),
            ..object(Kind::Matrix)
        };
        assert_eq!(
            convert("\u{fdd0}𝑎\u{fdee}𝑏\u{fdee}𝑐\u{fdee}𝑑\u{fdef}", vec![matrix]).unwrap(),
            "mat(delim: #none, a, b; c, d)"
        );
    }

    #[test]
    fn file_text_can_only_ever_be_symbols() {
        let source = convert("a,b;c$#\"/^_&@x->y", vec![]).unwrap();
        assert_eq!(source, "a \\, b \\; c \\$\\#\\\"\\/\\^\\_\\&\\@ x \\-\\> y");
        // Letters never merge into an unknown variable name.
        assert_eq!(convert("ab", vec![]).unwrap(), "a b");
    }

    #[test]
    fn malformed_structure_falls_back() {
        assert!(convert("\u{fdd0}a", vec![object(Kind::Fraction)]).is_none());
        assert!(convert("\u{fdd0}a\u{fdef}", vec![]).is_none());
        assert!(convert("a\u{fdef}b", vec![]).is_none());
        assert_eq!(plain("\u{fdd0}a\u{fdee}2\u{fdef}"), "a2");
        assert!(has_structure("x\u{fdd0}"));
    }

    #[test]
    fn mathematical_letters_map_to_their_base_letters() {
        assert_eq!(base_letter('𝑎'), 'a');
        assert_eq!(base_letter('𝐶'), 'C');
        assert_eq!(base_letter('𝑧'), 'z');
        assert_eq!(base_letter('ℎ'), 'ℎ');
        assert_eq!(base_letter('𝛾'), 'γ');
        assert_eq!(base_letter('𝜃'), 'θ');
        assert_eq!(base_letter('𝛤'), 'Γ');
    }
}
