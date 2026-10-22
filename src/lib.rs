//! OneNote sections into ordinary Goodtype notebooks. `goodtype-core` is the only canonical
//! writer; this crate decides what to write, never how notebooks are stored.

use std::{collections::HashSet, error::Error, fs, io::Read, path::Path};

use goodtype_core::{
    DEFAULT_INK_Z_INDEX, InkLayer, InkLayerReference, NotebookManifest, ObjectFields, Page,
    PageBackground, PageDefaults, PageGeometry, PageObject, PageReference, SCHEMA_VERSION, Stroke,
    StrokePattern, StrokePoint, StrokeTool, Transform, layout, paths,
    storage::{self, StoredFile},
};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub mod extract;
pub mod math;
pub mod raster;
pub mod source;
pub mod typst;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;
pub const MIB: usize = 1024 * 1024;
/// The upstream onenote.rs commit tagged v2.1.0, the exact `onenote_parser` release in use.
pub const DECODER_PIN: &str = "727389a674665dc07e0c31818cd4b54b64b9c684";
pub const DECODER: &str = "onenote_parser 2.1.0";
/// A OneNote section file this importer reads in full.
pub const MAX_SECTION_BYTES: usize = 512 * MIB;
const MAX_PAGES: usize = 2000;

#[derive(Debug, Serialize)]
pub struct PageReport {
    /// The Goodtype page number.
    pub page: usize,
    /// The OneNote page it came from, and its title.
    pub source_page: usize,
    pub title: Option<String>,
    pub typst_blocks: usize,
    pub strokes: usize,
    pub points: usize,
    pub images: usize,
    pub inline_images: usize,
    pub attachments: usize,
    /// Always empty: OneNote content is never turned into vector artwork. Present so readers of
    /// the shared report shape need no special case.
    pub derived: Vec<()>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct NotebookReport {
    pub source_sha256: String,
    pub decoder: String,
    pub decoder_commit: String,
    pub warnings: Vec<String>,
    pub pages: Vec<PageReport>,
}

pub fn read_limited(path: &Path, maximum: usize) -> Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > maximum as u64 {
        return Err(format!("file exceeds {} MiB: {}", maximum / MIB, path.display()).into());
    }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err("file grew beyond its size limit".into());
    }
    Ok(bytes)
}

/// A portable Library name for a notebook or folder.
pub fn safe_name(value: &str) -> String {
    let mut name: String = value
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(80)
        .collect();
    name = name.trim_matches([' ', '.']).to_owned();
    if name.is_empty() || paths::is_windows_reserved(&name) || name.starts_with('.') {
        name.insert_str(0, "Imported ");
    }
    name
}

/// One parsed OneNote page, cut into Goodtype pages.
pub struct SourcePage {
    pub title: Option<String>,
    pub pages: Vec<extract::PageModel>,
}

/// Parses a section's bytes into pages. Errors name the OneNote page that failed.
pub fn read_section(bytes: &[u8], file_name: &str) -> Result<(String, Vec<SourcePage>)> {
    let section = onenote_parser::Parser::new()
        .parse_section_buffer(bytes, typed_path::TypedPath::derive(file_name))
        .map_err(|error| format!("OneNote could not read this section ({error})"))?;
    let mut pages = Vec::new();
    let mut number = 0;
    for series in section.page_series() {
        for page in series.pages() {
            number += 1;
            pages.push(SourcePage {
                title: page.title_text().map(str::to_owned),
                pages: extract::page(page, number)?,
            });
        }
    }
    Ok((section.display_name().to_owned(), pages))
}

fn fields(id: String, x: f64, y: f64, z_index: i32, reading_order: u32, now: &str) -> ObjectFields {
    ObjectFields {
        id,
        x,
        y,
        rotation: 0.,
        scale: 1.,
        z_index,
        reading_order,
        group_id: None,
        created_at: now.into(),
        modified_at: now.into(),
    }
}

fn stroke(page_id: &str, number: usize, line: &extract::InkLine) -> Result<Stroke> {
    if !goodtype_core::valid_hex_color(&line.color) {
        return Err("invalid ink colour".into());
    }
    Ok(Stroke {
        id: layout::stroke_id(page_id, number),
        z_index: DEFAULT_INK_Z_INDEX,
        tool: if line.highlighter {
            StrokeTool::Highlighter
        } else {
            StrokeTool::Pen
        },
        color: line.color.clone(),
        width_pt: line.width_pt,
        pattern: StrokePattern::Solid,
        pattern_spacing: 1.,
        pressure: false,
        taper: 0.,
        opacity: line.opacity,
        group_id: None,
        points: line
            .points
            .iter()
            .enumerate()
            .map(|(n, &(x, y))| StrokePoint {
                x,
                y,
                pressure: 0.5,
                time_ms: n as f64,
                tilt_x: 0.,
                tilt_y: 0.,
            })
            .collect(),
        transform: Transform {
            translate_x: 0.,
            translate_y: 0.,
            scale_x: 1.,
            scale_y: 1.,
            rotation: 0.,
        },
    })
}

/// Writes `pages` as a new notebook at `destination` (which must not exist), keeps the section's
/// original bytes and attachments under `originals/`, and reopens every page through core.
pub fn convert(
    source_pages: Vec<SourcePage>,
    original: &[u8],
    destination: &Path,
    title: &str,
    warnings: Vec<String>,
) -> Result<NotebookReport> {
    if destination.exists() {
        return Err("staging notebook already exists".into());
    }
    let total: usize = source_pages
        .iter()
        .map(|page| page.pages.len().max(1))
        .sum();
    if total == 0 || total > MAX_PAGES {
        return Err(format!("expected 1–{MAX_PAGES} pages, found {total}").into());
    }
    fs::create_dir_all(destination)?;
    let now =
        time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
    let mut report = NotebookReport {
        source_sha256: format!("{:x}", Sha256::digest(original)),
        decoder: DECODER.into(),
        decoder_commit: DECODER_PIN.into(),
        warnings,
        pages: Vec::new(),
    };
    let mut attachments = Vec::new();
    let mut written = 0;
    for (source_index, source) in source_pages.into_iter().enumerate() {
        for (piece, mut model) in source.pages.into_iter().enumerate() {
            for (file, bytes) in &model.assets {
                let path = storage::store_pasted_image(destination, file, bytes)?;
                if path != layout::asset_path(file) {
                    return Err("picture stored outside the assets folder".into());
                }
            }
            attachments.append(&mut model.attachments);
            let geometry = PageGeometry {
                width_pt: model.width_pt,
                height_pt: model.height_pt,
            };
            let background = PageBackground::Plain {
                color: "#ffffff".into(),
            };
            let mut current = if written > 0 {
                storage::create_page(
                    destination,
                    &now,
                    &storage::PagePosition::Last,
                    Some(&background),
                    Some(&geometry),
                )?
            } else {
                let id = layout::page_id(1);
                storage::NotebookSnapshot {
                    manifest: NotebookManifest {
                        schema_version: SCHEMA_VERSION,
                        id: uuid::Uuid::new_v4().to_string(),
                        title: title.into(),
                        pages: vec![PageReference {
                            id: id.clone(),
                            path: layout::page_path(&id),
                            geometry: geometry.clone(),
                        }],
                        default_page: PageDefaults {
                            geometry: PageGeometry {
                                width_pt: 595.28,
                                height_pt: 841.89,
                            },
                            background: background.clone(),
                        },
                        shared_style_path: None,
                        created_at: now.clone(),
                        modified_at: now.clone(),
                    },
                    page: Page {
                        schema_version: SCHEMA_VERSION,
                        id,
                        revision: 1,
                        geometry,
                        background,
                        objects: vec![],
                        reading_order: vec![],
                        ink_layers: vec![],
                    },
                    blocks: vec![],
                    assets: vec![],
                    ink_layers: vec![],
                }
            };
            let page_id = current.page.id.clone();
            current.page.objects.clear();
            current.page.reading_order.clear();
            current.blocks.clear();
            // Background pictures first, then OneNote's own content order.
            enum Object<'a> {
                Block(&'a extract::TextBlock),
                Image(&'a extract::PlacedImage),
            }
            let mut objects: Vec<(usize, Object)> = model
                .blocks
                .iter()
                .map(|block| (block.order, Object::Block(block)))
                .chain(model.images.iter().map(|image| {
                    (
                        if image.background { 0 } else { image.order },
                        Object::Image(image),
                    )
                }))
                .collect();
            objects.sort_by_key(|(order, _)| *order);
            let mut block_number = 0;
            for (number, (_, object)) in objects.iter().enumerate() {
                let id = layout::object_id(&page_id, number + 1);
                let base = fields(id.clone(), 0., 0., number as i32 + 1, number as u32, &now);
                current.page.reading_order.push(id);
                current.page.objects.push(match object {
                    Object::Block(block) => {
                        block_number += 1;
                        let path = layout::block_path(&page_id, block_number);
                        current.blocks.push(StoredFile {
                            path: path.clone(),
                            bytes: block.source().into_bytes(),
                        });
                        PageObject::Typst {
                            fields: ObjectFields {
                                x: block.x,
                                y: block.y,
                                ..base
                            },
                            source_path: path,
                            layout_width_pt: block.width_pt,
                            measured_width_pt: block.width_pt,
                            measured_height_pt: block.height_pt(),
                        }
                    }
                    Object::Image(image) => PageObject::Image {
                        fields: ObjectFields {
                            x: image.x,
                            y: image.y,
                            ..base
                        },
                        source_path: storage::store_pasted_image(
                            destination,
                            &image.file,
                            &image.bytes,
                        )?,
                        width_pt: image.width_pt,
                        height_pt: image.height_pt,
                        alt_text: image.alt.clone(),
                    },
                });
            }
            let strokes = model
                .strokes
                .iter()
                .enumerate()
                .map(|(n, line)| stroke(&page_id, n + 1, line))
                .collect::<Result<Vec<_>>>()?;
            let layer_id = layout::ink_layer_id(&page_id, 1);
            current.page.ink_layers = vec![InkLayerReference {
                id: layer_id.clone(),
                path: layout::ink_layer_path(&page_id, 1),
            }];
            current.ink_layers = vec![InkLayer {
                schema_version: SCHEMA_VERSION,
                id: layer_id,
                page_id: page_id.clone(),
                strokes,
            }];
            if written == 0 {
                storage::create_notebook(destination, &current)?;
            } else {
                storage::save_notebook(destination, &current)?;
            }
            written += 1;
            report.pages.push(PageReport {
                page: written,
                source_page: source_index + 1,
                title: if piece == 0 {
                    source.title.clone()
                } else {
                    None
                },
                typst_blocks: model.blocks.len(),
                strokes: model.strokes.len(),
                points: model.strokes.iter().map(|s| s.points.len()).sum(),
                images: model.images.len(),
                inline_images: model.assets.len(),
                attachments: 0,
                derived: Vec::new(),
                warnings: model.warnings,
            });
        }
    }
    let originals = destination.join("originals");
    fs::create_dir(&originals)?;
    storage::files::persist_atomic(&originals.join("source.one"), original, false)?;
    if !attachments.is_empty() {
        let folder = originals.join("attachments");
        fs::create_dir(&folder)?;
        let mut taken = HashSet::new();
        for (index, (name, bytes)) in attachments.iter().enumerate() {
            let mut unique = name.clone();
            if !taken.insert(unique.to_lowercase()) {
                unique = format!("{} {name}", index + 1);
                taken.insert(unique.to_lowercase());
            }
            storage::files::persist_atomic(&folder.join(&unique), bytes, false)?;
        }
        if let Some(first) = report.pages.first_mut() {
            first.attachments = attachments.len();
        }
    }
    storage::files::persist_atomic(
        &destination.join("import-report.json"),
        &serde_json::to_vec_pretty(&report)?,
        false,
    )?;
    let reopened = storage::open_notebook(destination)?;
    for reference in &reopened.manifest.pages {
        storage::open_page(destination, &reference.id)?;
    }
    Ok(report)
}

/// Reserve a new notebook directory and publish its manifest last. No replacement operation.
pub fn publish(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    let root = paths::canonical_root(source)?;
    copy_tree(&root, &root, destination)?;
    Ok(())
}

fn copy_tree(root: &Path, source: &Path, destination: &Path) -> Result<()> {
    let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name() == layout::MANIFEST);
    for entry in entries {
        if entry.file_type()?.is_symlink() {
            return Err("symlink in staged notebook".into());
        }
        let path = paths::contained(root, &entry.path(), "publication source")?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            fs::create_dir(&target)?;
            copy_tree(root, &path, &target)?;
        } else {
            storage::files::persist_atomic(
                &target,
                &read_limited(&path, MAX_SECTION_BYTES)?,
                false,
            )?;
        }
    }
    Ok(())
}
