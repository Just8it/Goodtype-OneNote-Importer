//! OneNote pages into this importer's page model. Positions become page points here, once:
//! OneNote layout values are half-inches (36 pt) and ink is in 0.01 mm units.
//!
//! OneNote pages are an open canvas; a Goodtype page is fixed and loaded one at a time. A tall
//! OneNote page therefore becomes several A4-high Goodtype pages, cut only where nothing is
//! cut through: between strokes, pictures and text paragraphs, preferring wider empty bands.
//! A page grows past A4 only when a single piece (a picture, a drawing, one paragraph) leaves
//! no clean cut.

use std::{collections::BTreeMap, io::Read};

use onenote_parser::{
    contents::{
        Content, EmbeddedFile, EmbeddedObject, FileDataStatus, Image, Ink, MathInlineObject,
        MathObjectType, Outline, OutlineElement, OutlineItem, ParagraphStyling, RichText, Table,
    },
    page::{Page, PageContent},
    property::{common::ColorRef, embedded_file::FileType, rich_text::ParagraphAlignment},
};

use crate::{
    math, raster,
    typst::{self, Align, Item, ListKind, Paragraph, Run},
};

pub const HALF_INCH_PT: f64 = 36.0;
/// Ink coordinates and pen sizes are in 0.01 mm: 2540 units to the inch.
pub const INK_UNIT_PT: f64 = 72.0 / 2540.0;
const A4_PT: (f64, f64) = (595.28, 841.89);
const MARGIN_PT: f64 = 36.0;
const TITLE_SIZE_PT: f64 = 20.0;
/// Where OneNote draws a title whose anchor is the page origin.
const TITLE_ORIGIN_PT: (f64, f64) = (36.0, 14.0);
const MAX_ATTACHMENT_BYTES: usize = 128 * 1024 * 1024;
const MAX_INK_DEPTH: u32 = 16;

#[derive(Debug, Default)]
pub struct PageModel {
    pub width_pt: f64,
    pub height_pt: f64,
    pub blocks: Vec<TextBlock>,
    pub strokes: Vec<InkLine>,
    pub images: Vec<PlacedImage>,
    /// Pictures shown inside Typst blocks, referenced as `/assets/<file>`.
    pub assets: Vec<(String, Vec<u8>)>,
    /// Attached files, kept beside the notebook's original section.
    pub attachments: Vec<(String, Vec<u8>)>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TextBlock {
    pub x: f64,
    pub y: f64,
    pub width_pt: f64,
    pub items: Vec<Item>,
    /// Estimated height of each item; Goodtype measures the block when it first renders it.
    pub heights: Vec<f64>,
    pub base_size: Option<f64>,
    /// Position in the OneNote page's content order, for stacking and reading order.
    pub order: usize,
}

impl TextBlock {
    pub fn height_pt(&self) -> f64 {
        self.heights.iter().sum::<f64>().max(1.0)
    }

    pub fn source(&self) -> String {
        typst::block(&typst::items(&self.items), self.base_size)
    }
}

#[derive(Clone, Debug)]
pub struct InkLine {
    pub color: String,
    pub opacity: f64,
    pub width_pt: f64,
    pub highlighter: bool,
    pub points: Vec<(f64, f64)>,
}

#[derive(Clone, Debug)]
pub struct PlacedImage {
    pub file: String,
    pub bytes: Vec<u8>,
    pub x: f64,
    pub y: f64,
    pub width_pt: f64,
    pub height_pt: f64,
    pub alt: String,
    pub background: bool,
    pub order: usize,
}

fn half(value: Option<f32>) -> f64 {
    value.map_or(0.0, |value| f64::from(value) * HALF_INCH_PT)
}

fn math_object(object: &MathInlineObject) -> math::Object {
    use math::Kind;
    math::Object {
        kind: match object.object_type() {
            MathObjectType::Accent => Kind::Accent,
            MathObjectType::Brackets => Kind::Brackets,
            MathObjectType::BracketsWithSeps => Kind::BracketsWithSeps,
            MathObjectType::EquationArray => Kind::EquationArray,
            MathObjectType::Fraction => Kind::Fraction,
            MathObjectType::SlashedFraction => Kind::SlashedFraction,
            MathObjectType::FunctionApply => Kind::FunctionApply,
            MathObjectType::LeftSubSup => Kind::LeftSubSup,
            MathObjectType::LowerLimit => Kind::LowerLimit,
            MathObjectType::Matrix => Kind::Matrix,
            MathObjectType::Nary => Kind::Nary,
            MathObjectType::Overbar => Kind::Overbar,
            MathObjectType::Radical => Kind::Radical,
            MathObjectType::Stack => Kind::Stack,
            MathObjectType::Subscript => Kind::Subscript,
            MathObjectType::SubSup => Kind::SubSup,
            MathObjectType::Superscript => Kind::Superscript,
            MathObjectType::Underbar => Kind::Underbar,
            MathObjectType::UpperLimit => Kind::UpperLimit,
            _ => Kind::Other,
        },
        column: object.column(),
        char: object.char(),
        char1: object.char1(),
        char2: object.char2(),
    }
}

fn manual(color: Option<ColorRef>) -> Option<[u8; 3]> {
    match color? {
        ColorRef::Manual { r, g, b } => Some([r, g, b]),
        ColorRef::Auto => None,
    }
}

fn read_bounded(mut reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    (&mut reader)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read embedded data ({error})"))?;
    if bytes.len() > limit {
        return Err(format!(
            "embedded data exceeds {} MiB",
            limit / (1024 * 1024)
        ));
    }
    Ok(bytes)
}

/// Only web and mail links stay clickable; OneNote-internal and file links become plain text.
fn allowed_link(target: &str) -> bool {
    let lower = target.trim().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:"))
        && target.len() <= 2048
}

/// A portable attachment file name: no separators, reserved names or control characters.
pub fn safe_file_name(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control() && !"<>:\"|?*".contains(*c))
        .take(100)
        .collect();
    let cleaned = cleaned.trim_matches([' ', '.']).to_owned();
    let stem = cleaned.split('.').next().unwrap_or("");
    if cleaned.is_empty() || goodtype_core::paths::is_windows_reserved(stem) {
        fallback.to_owned()
    } else {
        cleaned
    }
}

struct Extractor {
    page: usize,
    pictures: usize,
    order: usize,
    model: PageModel,
    errors: Vec<String>,
    notes: BTreeMap<String, usize>,
}

/// Inline ink waiting for its container's width: (item index, ink, indent level).
type PendingInk<'p> = Vec<(usize, &'p Ink, usize)>;

impl Extractor {
    fn note(&mut self, note: impl Into<String>) {
        *self.notes.entry(note.into()).or_default() += 1;
    }

    fn next_order(&mut self) -> usize {
        self.order += 1;
        self.order
    }

    fn file_name(&mut self, extension: &str) -> String {
        self.pictures += 1;
        format!("p{:03}-img{:03}.{extension}", self.page, self.pictures)
    }

    /// Adds one text container; returns its estimated height.
    fn outline(&mut self, outline: &Outline, origin: (f64, f64), base_size: Option<f64>) -> f64 {
        let x = origin.0 + half(outline.offset_horizontal());
        let y = origin.1 + half(outline.offset_vertical());
        let mut items = Vec::new();
        let mut pending = Vec::new();
        self.outline_items(outline.items(), 0, &mut items, &mut pending);
        typst::number_lists(&mut items);
        let size = base_size.unwrap_or(typst::DEFAULT_SIZE_PT);
        let width = outline
            .layout_max_width()
            .filter(|width| *width > 0.0)
            .map(|width| f64::from(width) * HALF_INCH_PT)
            .unwrap_or_else(|| natural_width(&items, size))
            .clamp(36.0, 2000.0);
        for (index, ink, level) in pending {
            let offset = typst::estimate_height(&items[..index], width, size);
            self.note(
                "handwriting inside a text container was placed next to its text, approximately",
            );
            self.ink(ink, (x + level as f64 * 18.0, y + offset), false, 0);
        }
        if items.is_empty() {
            return 0.0;
        }
        let heights: Vec<f64> = items
            .iter()
            .map(|item| typst::estimate_item(item, width, size))
            .collect();
        let height = heights.iter().sum();
        let order = self.next_order();
        self.model.blocks.push(TextBlock {
            x,
            y,
            width_pt: width,
            items,
            heights,
            base_size,
            order,
        });
        height
    }

    fn outline_items<'p>(
        &mut self,
        items: &'p [OutlineItem],
        level: usize,
        out: &mut Vec<Item>,
        pending: &mut PendingInk<'p>,
    ) {
        for item in items {
            match item {
                OutlineItem::Element(element) => self.element(element, level, out, pending),
                OutlineItem::Group(group) => {
                    self.outline_items(group.outlines(), level, out, pending)
                }
            }
        }
    }

    fn element<'p>(
        &mut self,
        element: &'p OutlineElement,
        level: usize,
        out: &mut Vec<Item>,
        pending: &mut PendingInk<'p>,
    ) {
        // The list marker belongs to the element's first paragraph only.
        let mut list = element.list_contents().first().map(|list| {
            if list.list_format().contains(&'\u{fffd}') {
                ListKind::Numbered
            } else {
                ListKind::Bullet
            }
        });
        for content in element.contents() {
            match content {
                Content::RichText(text) => {
                    out.push(Item::Paragraph(self.paragraph(text, level, list.take())));
                    for object in text.embedded_objects() {
                        if let EmbeddedObject::Ink(container) = object {
                            pending.push((out.len() - 1, container.ink(), level));
                        }
                    }
                }
                Content::Table(table) => {
                    let item = self.table(table, level);
                    out.push(item);
                }
                Content::Image(image) => {
                    if let Some(item) = self.inline_image(image, level) {
                        out.push(item);
                    }
                }
                Content::EmbeddedFile(file) => {
                    let text = self.attachment(file);
                    out.push(Item::Note { level, text });
                }
                Content::Ink(ink) => pending.push((out.len(), ink, level)),
                Content::Unknown => self.note("content of an unknown type was skipped"),
            }
        }
        self.outline_items(element.children(), level + 1, out, pending);
    }

    fn paragraph(&mut self, text: &RichText, level: usize, list: Option<ListKind>) -> Paragraph {
        let utf16: Vec<u16> = text.text().encode_utf16().collect();
        let total = utf16.len() as u32;
        let base = text.paragraph_style();
        let styles = text.text_run_formatting();
        let indices = text.text_run_indices();
        // Same segmentation as the parser: run `i` ends at `indices[i]`, the last at the end.
        let mut segments: Vec<(u32, u32, &ParagraphStyling)> = Vec::new();
        if styles.is_empty() {
            segments.push((0, total, base));
        } else {
            let mut start = 0;
            for (index, style) in styles.iter().enumerate() {
                let end = indices
                    .get(index)
                    .copied()
                    .unwrap_or(total)
                    .min(total)
                    .max(start);
                segments.push((start, end, style));
                start = end;
            }
        }
        let links = text.hyperlinks();
        // One entry per text run: the run that opens an object carries the object's character
        // (`^`, `/`, `∮`, `(`, …); the others only say which argument a run belongs to.
        let objects: Vec<math::Object> = text
            .math_inline_objects()
            .iter()
            .filter(|object| object.char().is_some())
            .map(math_object)
            .collect();
        let mut next_object = 0;
        let mut equation = String::new();
        let mut runs = Vec::new();
        for (start, end, style) in segments {
            if style.hidden() || start >= end {
                continue;
            }
            let piece = String::from_utf16_lossy(&utf16[start as usize..end as usize]);
            if style.math_formatting() {
                equation.push_str(&piece);
                continue;
            }
            self.equation(&mut equation, &objects, &mut next_object, &mut runs);
            let link = links
                .iter()
                .find(|link| link.start() <= start && start < link.end())
                .map(|link| link.target().to_owned());
            let link = match link {
                Some(target) if allowed_link(&target) => Some(target),
                Some(_) => {
                    self.note("a link to a OneNote page or a file was kept as plain text");
                    None
                }
                None => None,
            };
            runs.push(Run {
                text: piece,
                bold: style.bold(),
                italic: style.italic(),
                underline: style.underline(),
                strike: style.strikethrough(),
                superscript: style.superscript(),
                subscript: style.subscript(),
                size_pt: style
                    .font_size()
                    .or(base.font_size())
                    .map(|size| f64::from(size) / 2.0),
                color: manual(style.font_color()),
                highlight: manual(style.highlight()),
                link,
                math: None,
            });
        }
        self.equation(&mut equation, &objects, &mut next_object, &mut runs);
        let checkbox = text.note_tags().iter().find_map(|tag| {
            let definition = tag.definition()?;
            format!("{:?}", definition.shape())
                .contains("CheckBox")
                .then(|| tag.item_status().completed())
        });
        if text.note_tags().iter().any(|tag| {
            tag.definition()
                .is_some_and(|d| !format!("{:?}", d.shape()).contains("CheckBox"))
        }) {
            self.note("a note tag other than a to-do box was not carried over");
        }
        Paragraph {
            runs,
            level,
            list,
            number: None,
            align: match text.paragraph_alignment() {
                ParagraphAlignment::Center => Align::Center,
                ParagraphAlignment::Right => Align::Right,
                _ => Align::Left,
            },
            checkbox,
        }
    }

    /// Ends a run of equation text: Typst math when its structure converts, plain text if not.
    fn equation(
        &mut self,
        equation: &mut String,
        objects: &[math::Object],
        next: &mut usize,
        runs: &mut Vec<Run>,
    ) {
        if equation.is_empty() {
            return;
        }
        // Each opening mark takes the next object, so a failed equation never shifts the next.
        let count = equation.matches('\u{fdd0}').count();
        let source = objects
            .get(*next..*next + count)
            .and_then(|own| math::to_typst(equation, &mut own.iter().copied()));
        *next += count;
        if source.is_none() {
            self.note("an equation could not be converted and was imported as text");
        }
        runs.push(Run {
            text: math::plain(equation),
            math: source,
            ..Run::default()
        });
        equation.clear();
    }

    fn table(&mut self, table: &Table, level: usize) -> Item {
        let mut rows = Vec::new();
        for row in table.contents() {
            let mut cells = Vec::new();
            for cell in row.contents() {
                let mut items = Vec::new();
                let mut pending = Vec::new();
                for element in cell.contents() {
                    self.element(element, 0, &mut items, &mut pending);
                }
                if !pending.is_empty() {
                    self.note("handwriting inside a table cell was not carried over");
                }
                cells.push(items);
            }
            rows.push(cells);
        }
        Item::Table {
            level,
            widths_pt: table
                .col_widths()
                .iter()
                .map(|width| f64::from(*width) * HALF_INCH_PT)
                .collect(),
            borders: table.borders_visible(),
            rows,
        }
    }

    /// The picture's bytes as Goodtype can show them, trying OneNote's own web-compatible copy
    /// when the primary format (for example EMF) is not displayable.
    fn picture(&mut self, image: &Image) -> Option<raster::Picture> {
        if image.data_status() != FileDataStatus::Available {
            self.note("a picture was not downloaded from OneDrive and is missing");
            return None;
        }
        let primary = image
            .read()
            .ok_or_else(|| "picture data is missing".to_owned())
            .and_then(|reader| read_bounded(reader, raster::MAX_IMAGE_BYTES))
            .and_then(raster::normalize);
        let error = match primary {
            Ok(picture) => return Some(picture),
            Err(error) => error,
        };
        if let Some(web) = image.web_picture()
            && web.data_status() == FileDataStatus::Available
            && let Ok(picture) =
                read_bounded(web.read(), raster::MAX_IMAGE_BYTES).and_then(raster::normalize)
        {
            return Some(picture);
        }
        self.note(format!("a picture was skipped: {error}"));
        None
    }

    fn image_size(image: &Image, picture: &raster::Picture) -> (f64, f64) {
        let natural = (
            f64::from(picture.width_px) * 0.75,
            f64::from(picture.height_px) * 0.75,
        );
        // `layout_max_*` is the size OneNote displays; `picture_*` the picture's own size.
        let width = image
            .layout_max_width()
            .or(image.picture_width())
            .filter(|width| *width > 0.0)
            .map(|width| f64::from(width) * HALF_INCH_PT);
        let height = image
            .layout_max_height()
            .or(image.picture_height())
            .filter(|height| *height > 0.0)
            .map(|height| f64::from(height) * HALF_INCH_PT);
        match (width, height) {
            (Some(width), Some(height)) => (width, height),
            (Some(width), None) => (width, width * natural.1 / natural.0),
            (None, Some(height)) => (height * natural.0 / natural.1, height),
            (None, None) => natural,
        }
    }

    fn inline_image(&mut self, image: &Image, level: usize) -> Option<Item> {
        let picture = self.picture(image)?;
        let (width_pt, height_pt) = Self::image_size(image, &picture);
        let file = self.file_name(picture.extension);
        self.model.assets.push((file.clone(), picture.bytes));
        Some(Item::Image {
            level,
            path: format!("/assets/{file}"),
            width_pt,
            height_pt,
        })
    }

    fn placed_image(&mut self, image: &Image) {
        let Some(picture) = self.picture(image) else {
            return;
        };
        let (width_pt, height_pt) = Self::image_size(image, &picture);
        let file = self.file_name(picture.extension);
        let order = self.next_order();
        let alt = image
            .alt_text()
            .map(|text| text.chars().take(500).collect())
            .unwrap_or_else(|| "Imported OneNote picture".to_owned());
        self.model.images.push(PlacedImage {
            file,
            bytes: picture.bytes,
            x: half(image.offset_horizontal()),
            y: half(image.offset_vertical()),
            width_pt,
            height_pt,
            alt,
            background: image.is_background(),
            order,
        });
    }

    /// Keeps the attached file and returns the label shown where it sat.
    fn attachment(&mut self, file: &EmbeddedFile) -> String {
        let name = safe_file_name(file.filename(), "attachment");
        if matches!(file.file_type(), FileType::Audio | FileType::Video) {
            self.note("an audio or video recording was kept as a file; Goodtype cannot play it");
        }
        if file.data_status() != FileDataStatus::Available {
            self.note("an attachment was not downloaded from OneDrive and is missing");
            return format!("Attachment {name} (missing in the OneNote export)");
        }
        match read_bounded(file.read(), MAX_ATTACHMENT_BYTES) {
            Ok(bytes) => {
                self.model.attachments.push((name.clone(), bytes));
                self.note("an attachment was kept in the notebook's originals/attachments folder");
                format!("Attachment: {name}")
            }
            Err(error) => {
                self.note(format!("an attachment was skipped: {error}"));
                format!("Attachment {name} (not imported)")
            }
        }
    }

    fn placed_attachment(&mut self, file: &EmbeddedFile) {
        let text = self.attachment(file);
        let item = Item::Note { level: 0, text };
        let order = self.next_order();
        let width_pt = 220.0;
        self.model.blocks.push(TextBlock {
            x: half(file.offset_horizontal()),
            y: half(file.offset_vertical()),
            width_pt,
            heights: vec![typst::estimate_item(
                &item,
                width_pt,
                typst::DEFAULT_SIZE_PT,
            )],
            items: vec![item],
            base_size: None,
            order,
        });
    }

    /// Ink at `origin`. Free-standing ink carries its own offset from the page; ink inside text
    /// is positioned by its container instead. Points are relative to the ink's bounding box.
    fn ink(&mut self, ink: &Ink, origin: (f64, f64), own_offset: bool, depth: u32) {
        if depth > MAX_INK_DEPTH {
            self.errors.push("ink is nested too deeply".into());
            return;
        }
        let base = if own_offset {
            (
                origin.0 + half(ink.offset_horizontal()),
                origin.1 + half(ink.offset_vertical()),
            )
        } else {
            origin
        };
        for child in ink.child_groups() {
            self.ink(child, base, own_offset, depth + 1);
        }
        let anchor = ink.bounding_box().map_or((0.0, 0.0), |bounds| {
            (f64::from(bounds.x()), f64::from(bounds.y()))
        });
        for stroke in ink.ink_strokes() {
            let points: Vec<(f64, f64)> = stroke
                .absolute_path()
                .iter()
                .map(|point| {
                    (
                        base.0 + (f64::from(point.x()) - anchor.0) * INK_UNIT_PT,
                        base.1 + (f64::from(point.y()) - anchor.1) * INK_UNIT_PT,
                    )
                })
                .collect();
            if points.is_empty() || points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
                self.note("an ink stroke without usable points was skipped");
                continue;
            }
            let size = f64::from(stroke.width().max(stroke.height())) * INK_UNIT_PT;
            let width_pt = if size.is_finite() && size > 0.0 {
                size.clamp(0.1, 64.0)
            } else {
                1.0
            };
            let transparency = f64::from(stroke.transparency().unwrap_or(0));
            let color = stroke.color().unwrap_or(0);
            self.model.strokes.push(InkLine {
                color: format!(
                    "#{:02x}{:02x}{:02x}",
                    color & 0xff,
                    (color >> 8) & 0xff,
                    (color >> 16) & 0xff
                ),
                opacity: (1.0 - transparency / 255.0).clamp(0.05, 1.0),
                width_pt,
                highlighter: transparency >= 64.0,
                points,
            });
        }
    }

    /// The page's content, moved into view and cut into Goodtype pages.
    fn finish(mut self) -> Result<Vec<PageModel>, String> {
        if let Some(error) = self.errors.first() {
            return Err(format!("page {}: {error}", self.page));
        }
        self.model.warnings = self
            .notes
            .into_iter()
            .map(|(note, count)| {
                if count == 1 {
                    note
                } else {
                    format!("{note} ({count}×)")
                }
            })
            .collect();
        into_view(&mut self.model);
        let pages = paginate(self.model, A4_PT);
        for page in &pages {
            let limit = goodtype_core::MAX_PAGE_DIMENSION_PT;
            if page.width_pt > limit || page.height_pt > limit {
                return Err(format!(
                    "page {} holds a piece larger than a Goodtype page may be",
                    self.page
                ));
            }
            if page
                .blocks
                .iter()
                .any(|block| block.source().len() > goodtype_core::object::MAX_BLOCK_BYTES)
            {
                return Err(format!(
                    "page {} has a paragraph larger than a Typst block may be (1 MiB)",
                    self.page
                ));
            }
        }
        Ok(pages)
    }
}

/// Content that OneNote placed left of or above the page origin moves into view.
fn into_view(model: &mut PageModel) {
    let mut min = (f64::INFINITY, f64::INFINITY);
    for block in &model.blocks {
        min = (min.0.min(block.x), min.1.min(block.y));
    }
    for image in &model.images {
        min = (min.0.min(image.x), min.1.min(image.y));
    }
    for stroke in &model.strokes {
        for &(x, y) in &stroke.points {
            min = (
                min.0.min(x - stroke.width_pt),
                min.1.min(y - stroke.width_pt),
            );
        }
    }
    let shift = (
        if min.0 < 0.0 { MARGIN_PT - min.0 } else { 0.0 },
        if min.1 < 0.0 { MARGIN_PT - min.1 } else { 0.0 },
    );
    if shift == (0.0, 0.0) {
        return;
    }
    for block in &mut model.blocks {
        block.x += shift.0;
        block.y += shift.1;
    }
    for image in &mut model.images {
        image.x += shift.0;
        image.y += shift.1;
    }
    for stroke in &mut model.strokes {
        for point in &mut stroke.points {
            point.0 += shift.0;
            point.1 += shift.1;
        }
    }
}

/// Something that must stay whole on one page: a stroke, a picture, or one item (paragraph,
/// list entry, table, inline picture) of a text container.
#[derive(Clone, Copy, Debug)]
enum Piece {
    Item { block: usize, item: usize },
    Stroke(usize),
    Image(usize),
}

#[derive(Clone, Copy, Debug)]
struct Unit {
    top: f64,
    bottom: f64,
    piece: Piece,
}

fn units(model: &PageModel) -> Vec<Unit> {
    let mut units = Vec::new();
    for (index, block) in model.blocks.iter().enumerate() {
        let mut top = block.y;
        for (item, height) in block.heights.iter().enumerate() {
            units.push(Unit {
                top,
                bottom: top + height,
                piece: Piece::Item { block: index, item },
            });
            top += height;
        }
    }
    for (index, stroke) in model.strokes.iter().enumerate() {
        let (low, high) = stroke
            .points
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                (low.min(point.1), high.max(point.1))
            });
        let half = stroke.width_pt / 2.0;
        units.push(Unit {
            top: low - half,
            bottom: high + half,
            piece: Piece::Stroke(index),
        });
    }
    for (index, image) in model.images.iter().enumerate() {
        units.push(Unit {
            top: image.y,
            bottom: image.y + image.height_pt,
            piece: Piece::Image(index),
        });
    }
    units.sort_by(|a, b| a.top.total_cmp(&b.top).then(a.bottom.total_cmp(&b.bottom)));
    units
}

/// Index of the first unit on every page. A cut sits after unit `i` only when every unit up to
/// `i` ends before the next one begins, so nothing is ever cut through. `block_items` is the
/// item count of every text container.
fn page_starts(units: &[Unit], block_items: &[usize], page_height: f64) -> Vec<usize> {
    const TOUCHING: f64 = 0.5;
    // Splitting one text container into two blocks costs about eight lines of page fill.
    const SPLIT_COST: f64 = 120.0;
    let mut starts = vec![0];
    let mut start = 0;
    let mut page_top = 0.0;
    while start < units.len() {
        let limit = page_top + page_height - MARGIN_PT;
        let fuller = page_top + page_height * 0.5;
        // (score, next start) for cuts in the lower half, then anywhere, then past the limit.
        let mut lower: Option<(f64, usize)> = None;
        let mut anywhere: Option<(f64, usize)> = None;
        let mut overflow = None;
        let mut covered = f64::NEG_INFINITY;
        // Containers with items above the cut, and the last such item.
        let mut open: BTreeMap<usize, usize> = BTreeMap::new();
        for index in start..units.len() {
            if units[index].top > limit && (lower.is_some() || anywhere.is_some()) {
                break;
            }
            covered = covered.max(units[index].bottom);
            if let Piece::Item { block, item } = units[index].piece {
                let last = open.entry(block).or_insert(item);
                *last = (*last).max(item);
            }
            let next = units.get(index + 1).map_or(f64::INFINITY, |unit| unit.top);
            if next < covered - TOUCHING {
                continue;
            }
            if covered > limit {
                overflow = Some(index + 1);
                break;
            }
            // Wider empty bands make better cuts and keeping containers whole is better still;
            // a fuller page is preferred otherwise.
            let splits = open
                .iter()
                .any(|(block, last)| last + 1 < block_items[*block]);
            let score =
                covered + 2.0 * (next - covered).min(72.0) - if splits { SPLIT_COST } else { 0.0 };
            let slot = if covered >= fuller {
                &mut lower
            } else {
                &mut anywhere
            };
            if slot.is_none_or(|(best, _)| score >= best) {
                *slot = Some((score, index + 1));
            }
        }
        let Some(next) = lower.or(anywhere).map(|(_, next)| next).or(overflow) else {
            break;
        };
        if next >= units.len() {
            break;
        }
        starts.push(next);
        start = next;
        page_top = units[next].top - MARGIN_PT;
    }
    starts
}

/// Cuts one OneNote page into Goodtype pages of `size`. Later pages move their content up so it
/// starts one margin below the top; the first keeps OneNote's own positions.
fn paginate(mut model: PageModel, size: (f64, f64)) -> Vec<PageModel> {
    let units = units(&model);
    let block_items: Vec<usize> = model.blocks.iter().map(|block| block.items.len()).collect();
    let starts = page_starts(&units, &block_items, size.1);
    let mut pages = Vec::new();
    for (number, &start) in starts.iter().enumerate() {
        let end = starts.get(number + 1).copied().unwrap_or(units.len());
        let shift = if number == 0 {
            0.0
        } else {
            units[start].top - MARGIN_PT
        };
        let mut page = PageModel::default();
        if number == 0 {
            page.assets = std::mem::take(&mut model.assets);
            page.attachments = std::mem::take(&mut model.attachments);
            page.warnings = std::mem::take(&mut model.warnings);
        }
        let mut items: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
        for unit in &units[start..end] {
            match unit.piece {
                Piece::Item { block, item } => {
                    let range = items.entry(block).or_insert((item, item));
                    *range = (range.0.min(item), range.1.max(item));
                }
                Piece::Stroke(index) => {
                    let mut stroke = model.strokes[index].clone();
                    for point in &mut stroke.points {
                        point.1 -= shift;
                    }
                    page.strokes.push(stroke);
                }
                Piece::Image(index) => {
                    let mut image = model.images[index].clone();
                    image.y -= shift;
                    page.images.push(image);
                }
            }
        }
        for (block, (first, last)) in items {
            let source = &model.blocks[block];
            page.blocks.push(TextBlock {
                x: source.x,
                y: source.y + source.heights[..first].iter().sum::<f64>() - shift,
                width_pt: source.width_pt,
                items: source.items[first..=last].to_vec(),
                heights: source.heights[first..=last].to_vec(),
                base_size: source.base_size,
                order: source.order,
            });
        }
        let mut right: f64 = 0.0;
        let mut bottom: f64 = 0.0;
        for block in &page.blocks {
            right = right.max(block.x + block.width_pt);
            bottom = bottom.max(block.y + block.height_pt());
        }
        for image in &page.images {
            right = right.max(image.x + image.width_pt);
            bottom = bottom.max(image.y + image.height_pt);
        }
        for stroke in &page.strokes {
            for &(x, y) in &stroke.points {
                right = right.max(x + stroke.width_pt);
                bottom = bottom.max(y + stroke.width_pt);
            }
        }
        page.width_pt = (right + MARGIN_PT).max(size.0);
        page.height_pt = (bottom + MARGIN_PT).max(size.1);
        pages.push(page);
    }
    pages
}

/// Width for a container OneNote let grow with its text: the longest paragraph, capped at a
/// comfortable line length.
fn natural_width(items: &[Item], size: f64) -> f64 {
    items
        .iter()
        .map(|item| match item {
            Item::Paragraph(p) => {
                let chars = p
                    .runs
                    .iter()
                    .map(|run| run.text.chars().count())
                    .sum::<usize>();
                chars as f64 * 0.5 * size + p.level as f64 * 18.0 + 4.0
            }
            Item::Table { widths_pt, .. } => widths_pt.iter().sum::<f64>() + 8.0,
            Item::Image { width_pt, .. } => *width_pt,
            Item::Note { text, .. } => text.chars().count() as f64 * 0.5 * size + 16.0,
        })
        .fold(72.0, f64::max)
        .min(540.0)
}

/// One OneNote page as one or more Goodtype pages of page-point content.
pub fn page(page: &Page, number: usize) -> Result<Vec<PageModel>, String> {
    let mut extractor = Extractor {
        page: number,
        pictures: 0,
        order: 0,
        model: PageModel::default(),
        errors: Vec::new(),
        notes: BTreeMap::new(),
    };
    if let Some(title) = page.title() {
        // The title and its date/time are separate outlines without offsets of their own;
        // OneNote stacks them from the title's anchor, which is usually the page origin.
        let x = (f64::from(title.offset_horizontal()) * HALF_INCH_PT).max(TITLE_ORIGIN_PT.0);
        let mut y = (f64::from(title.offset_vertical()) * HALF_INCH_PT).max(TITLE_ORIGIN_PT.1);
        for (index, outline) in title.contents().iter().enumerate() {
            let size = (index == 0).then_some(TITLE_SIZE_PT);
            if outline.offset_horizontal().is_some() || outline.offset_vertical().is_some() {
                extractor.outline(outline, (x, y), size);
            } else {
                y += extractor.outline(outline, (x, y), size);
            }
        }
    }
    for content in page.contents() {
        match content {
            PageContent::Outline(outline) => {
                extractor.outline(outline, (0.0, 0.0), None);
            }
            PageContent::Image(image) => extractor.placed_image(image),
            PageContent::EmbeddedFile(file) => extractor.placed_attachment(file),
            PageContent::Ink(ink) => extractor.ink(ink, (0.0, 0.0), true, 0),
            PageContent::Unknown => extractor.note("content of an unknown type was skipped"),
        }
    }
    extractor.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typst::Paragraph;

    fn lines(count: usize, height: f64) -> TextBlock {
        TextBlock {
            x: 36.0,
            y: 36.0,
            width_pt: 400.0,
            items: (0..count)
                .map(|n| {
                    Item::Paragraph(Paragraph {
                        runs: vec![Run {
                            text: format!("line {n}"),
                            ..Run::default()
                        }],
                        ..Paragraph::default()
                    })
                })
                .collect(),
            heights: vec![height; count],
            base_size: None,
            order: 1,
        }
    }

    fn stroke(top: f64, bottom: f64) -> InkLine {
        InkLine {
            color: "#000000".into(),
            opacity: 1.0,
            width_pt: 1.0,
            highlighter: false,
            points: vec![(100.0, top), (120.0, bottom)],
        }
    }

    fn picture(y: f64, height: f64) -> PlacedImage {
        PlacedImage {
            file: "p001-img001.png".into(),
            bytes: vec![1],
            x: 36.0,
            y,
            width_pt: 100.0,
            height_pt: height,
            alt: String::new(),
            background: false,
            order: 2,
        }
    }

    fn tops(page: &PageModel) -> Vec<f64> {
        let mut tops: Vec<f64> = page.blocks.iter().map(|b| b.y).collect();
        tops.extend(page.images.iter().map(|i| i.y));
        tops.extend(page.strokes.iter().map(|s| s.points[0].1));
        tops
    }

    #[test]
    fn a_short_page_stays_one_a4_page() {
        let model = PageModel {
            blocks: vec![lines(10, 15.0)],
            strokes: vec![stroke(400.0, 420.0)],
            ..PageModel::default()
        };
        let pages = paginate(model, A4_PT);
        assert_eq!(pages.len(), 1);
        assert_eq!((pages[0].width_pt, pages[0].height_pt), A4_PT);
        assert_eq!(pages[0].blocks[0].items.len(), 10);
    }

    #[test]
    fn long_text_splits_between_paragraphs_and_keeps_counting() {
        let mut block = lines(150, 15.0);
        for item in &mut block.items {
            if let Item::Paragraph(p) = item {
                p.list = Some(ListKind::Numbered);
            }
        }
        typst::number_lists(&mut block.items);
        let pages = paginate(
            PageModel {
                blocks: vec![block],
                ..PageModel::default()
            },
            A4_PT,
        );
        assert_eq!(pages.len(), 3);
        let counts: Vec<usize> = pages.iter().map(|p| p.blocks[0].items.len()).collect();
        assert_eq!(counts.iter().sum::<usize>(), 150);
        for page in &pages {
            assert_eq!(page.height_pt, A4_PT.1, "no page grows");
            let block = &page.blocks[0];
            assert!(block.y + block.height_pt() <= A4_PT.1 - MARGIN_PT + 0.01);
        }
        // Later pages start one margin from the top and continue the numbering.
        assert_eq!(pages[1].blocks[0].y, MARGIN_PT);
        assert!(
            pages[1].blocks[0]
                .source()
                .contains(&format!("{}. line", counts[0] + 1))
        );
    }

    #[test]
    fn cuts_between_strokes_never_through_one() {
        // Handwriting lines 30 pt apart down 2000 pt; every stroke must land whole on a page.
        let strokes: Vec<InkLine> = (0..60)
            .map(|line| {
                let top = 50.0 + line as f64 * 33.0;
                stroke(top, top + 20.0)
            })
            .collect();
        let pages = paginate(
            PageModel {
                strokes,
                ..PageModel::default()
            },
            A4_PT,
        );
        assert!(pages.len() >= 3);
        assert_eq!(pages.iter().map(|p| p.strokes.len()).sum::<usize>(), 60);
        for page in &pages {
            assert_eq!(page.height_pt, A4_PT.1);
            for stroke in &page.strokes {
                assert!(stroke.points.iter().all(|p| p.1 >= 0.0 && p.1 <= A4_PT.1));
            }
        }
    }

    #[test]
    fn prefers_gaps_between_containers_to_splitting_one() {
        // Three containers: 36..486, 560..770 and 772..1072. Splitting the third after two
        // lines would fill the page further; ending at the second container keeps all whole.
        let mut first = lines(30, 15.0);
        first.order = 1;
        let mut second = lines(14, 15.0);
        second.y = 560.0;
        second.order = 2;
        let mut third = lines(20, 15.0);
        third.y = 772.0;
        third.order = 3;
        let pages = paginate(
            PageModel {
                blocks: vec![first, second, third],
                ..PageModel::default()
            },
            A4_PT,
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].blocks.len(), 2);
        assert_eq!(pages[1].blocks.len(), 1);
        assert_eq!(pages[1].blocks[0].items.len(), 20);
        assert_eq!(pages[1].blocks[0].y, MARGIN_PT);
    }

    #[test]
    fn prefers_a_wide_band_to_a_tight_one_lower_down() {
        // 22 handwriting lines to 690 pt, a 71 pt band, then lines 5 pt apart.
        let mut strokes: Vec<InkLine> = (0..22)
            .map(|line| stroke(40.0 + line as f64 * 30.0, 60.0 + line as f64 * 30.0))
            .collect();
        strokes.extend((0..30).map(|line| {
            let top = 762.0 + line as f64 * 25.5;
            stroke(top, top + 20.0)
        }));
        let pages = paginate(
            PageModel {
                strokes,
                ..PageModel::default()
            },
            A4_PT,
        );
        // Cutting in the band (690 + 2·71) beats one more line down to 782 (782 + 2·5).
        assert_eq!(pages[0].strokes.len(), 22);
        assert_eq!(pages.iter().map(|p| p.strokes.len()).sum::<usize>(), 52);
    }

    #[test]
    fn a_piece_taller_than_a_page_grows_only_its_page() {
        let pages = paginate(
            PageModel {
                images: vec![picture(36.0, 1500.0)],
                blocks: vec![{
                    let mut block = lines(5, 15.0);
                    block.y = 1600.0;
                    block
                }],
                ..PageModel::default()
            },
            A4_PT,
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].images.len(), 1);
        assert!(pages[0].height_pt >= 1536.0);
        assert_eq!(pages[1].height_pt, A4_PT.1);
        assert_eq!(tops(&pages[1]), [MARGIN_PT]);
    }

    #[test]
    fn content_left_of_or_above_the_origin_moves_into_view() {
        let mut model = PageModel {
            strokes: vec![stroke(-20.0, 10.0)],
            ..PageModel::default()
        };
        into_view(&mut model);
        assert!(
            model.strokes[0]
                .points
                .iter()
                .all(|p| p.1 >= MARGIN_PT - 1.0)
        );
    }
}
