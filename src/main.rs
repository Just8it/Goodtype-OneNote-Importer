use std::{
    collections::{HashMap, HashSet},
    env, fs,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
};

use goodtype_core::{paths, storage::files::persist_atomic};
use goodtype_onenote_import::{
    DECODER_PIN, MAX_SECTION_BYTES, Result, convert, publish, read_limited, read_section,
    safe_name, source,
};
use serde::Serialize;

const NAME: &str = "goodtype-onenote-import";
const USAGE: &str = "usage: goodtype-onenote-import <section.one|notebook.onetoc2|package.onepkg|export.zip> <NEW destination folder> [--allow-derived] [--title <single notebook title>]";

#[derive(Serialize)]
struct Outcome {
    status: &'static str,
    source: String,
    target: Option<String>,
    pages: usize,
    error: Option<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("OneNote import: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.first().is_some_and(|arg| arg == "--version") {
        if arguments.len() == 2 && arguments[1] == "--json" {
            println!(
                "{}",
                serde_json::json!({"name": NAME, "version": env!("CARGO_PKG_VERSION"), "protocolVersion": 1, "notebookSchemaVersion": goodtype_core::SCHEMA_VERSION, "decoderCommit": DECODER_PIN})
            );
        } else if arguments.len() == 1 {
            println!("Goodtype OneNote Importer {}", env!("CARGO_PKG_VERSION"));
        } else {
            return Err("usage: goodtype-onenote-import --version [--json]".into());
        }
        return Ok(());
    }
    if arguments.len() == 1 && arguments[0] == "--check" {
        // No separate runtime: the decoder is compiled in. Prove the writer and parser link.
        println!(
            "{} {DECODER_PIN}; goodtype-core schema {}",
            goodtype_onenote_import::DECODER,
            goodtype_core::SCHEMA_VERSION
        );
        return Ok(());
    }
    if arguments.len() == 1 && (arguments[0] == "--help" || arguments[0] == "-h") {
        println!(
            "Goodtype OneNote Importer\n\n{USAGE}\n\n--allow-derived  Accepted for compatibility; OneNote content never needs vector artwork\n--version --json Print the version and compatibility contract\n--check          Verify the importer\n\nConversion stays local. Each notebook keeps its original section file."
        );
        return Ok(());
    }
    let mut args = env::args_os().skip(1);
    let source = args.next().ok_or(USAGE)?;
    let destination = PathBuf::from(args.next().ok_or(USAGE)?);
    let source = fs::canonicalize(source)?;
    let mut title = None;
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--allow-derived") => {}
            Some("--title") => {
                title = Some(
                    args.next()
                        .ok_or(USAGE)?
                        .into_string()
                        .map_err(|_| "title is not Unicode")?,
                )
            }
            _ => return Err(USAGE.into()),
        }
    }
    if destination.try_exists()? {
        return Err("destination already exists; choose a new batch folder".into());
    }
    let parent = paths::canonical_root(
        destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    let name = destination
        .file_name()
        .ok_or("destination needs a folder name")?;
    let destination = parent.join(name);
    let scratch = tempfile::Builder::new()
        .prefix(".onenote-import-")
        .tempdir_in(&parent)?;
    let jobs = source::inventory(&source, scratch.path())?;
    if title.is_some() && jobs.len() != 1 {
        return Err("--title is only for a single section".into());
    }
    // Resolve every target before creating the destination or converting anything.
    let mut targets = HashSet::new();
    let mut folder_names = HashMap::new();
    let mut planned = Vec::new();
    for (index, job) in jobs.into_iter().enumerate() {
        let mut relative = PathBuf::new();
        for folder in &job.folders {
            relative.push(safe_name(folder));
            let key = relative.to_string_lossy().to_lowercase();
            if folder_names
                .insert(key, folder.clone())
                .is_some_and(|old| old != *folder)
            {
                return Err("folder names collide after portable-name normalization".into());
            }
        }
        let stem = safe_name(&job.stem);
        let mut candidate = relative.join(&stem);
        if !targets.insert(candidate.to_string_lossy().to_lowercase()) {
            candidate = relative.join(format!("{stem} ({})", index + 1));
            if !targets.insert(candidate.to_string_lossy().to_lowercase()) {
                return Err("section names collide".into());
            }
        }
        planned.push((job, candidate));
    }
    for (_, relative) in &planned {
        let key = relative.to_string_lossy().to_lowercase();
        if folder_names.contains_key(&key) || key == "import-report.json" {
            return Err("section and folder/report names collide".into());
        }
    }
    fs::create_dir(&destination)?;
    let destination_root = paths::canonical_root(&destination)?;
    let report_path = destination_root.join("import-report.json");
    let mut outcomes: Vec<_> = planned
        .iter()
        .map(|(job, relative)| Outcome {
            status: "pending",
            source: job.label.clone(),
            target: Some(relative.to_string_lossy().replace('\\', "/")),
            pages: 0,
            error: None,
        })
        .collect();
    persist_atomic(&report_path, &serde_json::to_vec_pretty(&outcomes)?, false)?;
    for (index, (job, relative)) in planned.into_iter().enumerate() {
        println!("Importing {}", job.label);
        let result = (|| -> Result<usize> {
            let bytes = read_limited(&job.file, MAX_SECTION_BYTES)?;
            let file_name = format!("{}.one", job.stem);
            // A malformed section must fail itself, never the whole batch.
            let (display_name, pages) =
                panic::catch_unwind(AssertUnwindSafe(|| read_section(&bytes, &file_name)))
                    .map_err(|_| "the OneNote section could not be decoded")??;
            let notebook_title = title.clone().unwrap_or_else(|| {
                if display_name.trim().is_empty() {
                    job.stem.clone()
                } else {
                    display_name
                }
            });
            let job_dir = tempfile::Builder::new()
                .prefix("notebook-")
                .tempdir_in(scratch.path())?;
            let staged = job_dir.path().join("notebook");
            let report = convert(pages, &bytes, &staged, &notebook_title, Vec::new())?;
            let target = destination_root.join(&relative);
            let folder = target.parent().ok_or("target has no parent")?;
            fs::create_dir_all(folder)?;
            paths::contained(&destination_root, folder, "destination folder")?;
            publish(&staged, &target)?;
            Ok(report.pages.len())
        })();
        outcomes[index] = match result {
            Ok(pages) => {
                println!("Imported {pages} pages into {}", relative.display());
                Outcome {
                    status: "completed",
                    source: job.label,
                    target: Some(relative.to_string_lossy().replace('\\', "/")),
                    pages,
                    error: None,
                }
            }
            Err(error) => {
                eprintln!("Failed {}: {error}", job.label);
                Outcome {
                    status: "failed",
                    source: job.label,
                    target: None,
                    pages: 0,
                    error: Some(error.to_string()),
                }
            }
        };
        persist_atomic(&report_path, &serde_json::to_vec_pretty(&outcomes)?, true)?;
    }
    if outcomes.iter().any(|o| o.error.is_some()) {
        return Err(format!(
            "some sections failed; completed notebooks are preserved. See {}",
            report_path.display()
        )
        .into());
    }
    println!(
        "Done. Open {} in the Goodtype Library.",
        destination_root.display()
    );
    Ok(())
}
