//! What the user picked, as a list of OneNote sections. Every section becomes one Goodtype
//! notebook; notebooks and section groups become Library folders.
//!
//! Accepted: one section (`.one`), a notebook folder through its `.onetoc2`, a `.onepkg` package
//! from OneNote for Windows (a CAB archive) and a OneNote-for-the-web ZIP export. Archives are
//! unpacked into scratch space with the same rules everywhere: only `.one` files, portable
//! relative names, no links, no duplicates, and sizes counted as bytes are written.

use std::{
    collections::HashSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use crate::Result;

const MAX_ENTRIES: usize = 10_000;
const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DEPTH: usize = 12;
pub const MAX_SECTIONS: usize = 500;
const RECYCLE_BIN: &str = "onenote_recyclebin";

#[derive(Debug, PartialEq)]
pub struct Job {
    /// Library folders above the notebook: OneNote notebook, then section groups.
    pub folders: Vec<String>,
    pub file: PathBuf,
    /// The section's file name without `.one`.
    pub stem: String,
    /// Where the section sat in the export, for the report.
    pub label: String,
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One archive path component that is safe as a file name on every platform.
fn safe_component(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    !(component.is_empty()
        || component == "."
        || component == ".."
        || component.ends_with(['.', ' '])
        || component.chars().any(char::is_control)
        || component.contains(['<', '>', ':', '"', '|', '?', '*', '/', '\\'])
        || goodtype_core::paths::is_windows_reserved(stem))
}

fn archive_components(name: &str) -> Result<Vec<String>> {
    let components: Vec<String> = name
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    if components.is_empty()
        || components.len() > MAX_DEPTH + 2
        || name.starts_with(['/', '\\'])
        || !components.iter().all(|part| safe_component(part))
    {
        return Err(format!("unsafe path `{name}` in the archive").into());
    }
    Ok(components)
}

fn is_section(components: &[String]) -> bool {
    components
        .last()
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".one"))
        && !components.iter().any(|part| {
            part.eq_ignore_ascii_case(RECYCLE_BIN) || part.eq_ignore_ascii_case("__MACOSX")
        })
}

/// Writes one archive member below `into`, within the remaining byte budget.
fn unpack(
    into: &Path,
    components: &[String],
    reader: &mut dyn Read,
    seen: &mut HashSet<String>,
    written: &mut u64,
) -> Result<()> {
    if !seen.insert(components.join("/").to_lowercase()) {
        return Err(format!("the archive lists `{}` twice", components.join("/")).into());
    }
    let target = components
        .iter()
        .fold(into.to_path_buf(), |path, part| path.join(part));
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)?;
    let budget = MAX_UNPACKED_BYTES - *written;
    *written += io::copy(&mut reader.take(budget + 1), &mut output)?;
    if *written > MAX_UNPACKED_BYTES {
        return Err("the archive unpacks to more than 512 MiB".into());
    }
    Ok(())
}

pub fn extract_zip(archive: &Path, into: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)
        .map_err(|error| format!("not a valid ZIP ({error})"))?;
    if zip.len() > MAX_ENTRIES {
        return Err("the ZIP has too many entries".into());
    }
    fs::create_dir(into)?;
    let mut seen = HashSet::new();
    let mut written = 0;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        if entry.is_symlink() || entry.encrypted() {
            return Err(format!("the ZIP entry `{}` is a link or encrypted", entry.name()).into());
        }
        let components = archive_components(entry.name())?;
        if is_section(&components) {
            unpack(into, &components, &mut entry, &mut seen, &mut written)?;
        }
    }
    Ok(())
}

pub fn extract_cab(archive: &Path, into: &Path) -> Result<()> {
    let mut cabinet = cab::Cabinet::new(fs::File::open(archive)?)
        .map_err(|error| format!("not a valid OneNote package ({error})"))?;
    let names: Vec<String> = cabinet
        .folder_entries()
        .flat_map(|folder| folder.file_entries())
        .map(|entry| entry.name().to_owned())
        .collect();
    if names.len() > MAX_ENTRIES {
        return Err("the OneNote package has too many entries".into());
    }
    fs::create_dir(into)?;
    let mut seen = HashSet::new();
    let mut written = 0;
    for name in names {
        let components = archive_components(&name)?;
        if is_section(&components) {
            let mut reader = cabinet.read_file(&name)?;
            unpack(into, &components, &mut reader, &mut seen, &mut written)?;
        }
    }
    Ok(())
}

/// Every section below `root`, in name order, with section-group folders appended to `folders`.
fn walk(root: &Path, folders: &[String], jobs: &mut Vec<Job>, label: &str) -> Result<()> {
    if folders.len() > MAX_DEPTH {
        return Err("section groups are nested more than 12 levels deep".into());
    }
    let mut entries = fs::read_dir(root)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
    for entry in entries {
        let kind = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if kind.is_symlink() || name.eq_ignore_ascii_case(RECYCLE_BIN) {
            continue;
        }
        let path = entry.path();
        let label = if label.is_empty() {
            name.clone()
        } else {
            format!("{label}/{name}")
        };
        if kind.is_dir() {
            let mut inner = folders.to_vec();
            inner.push(name);
            walk(&path, &inner, jobs, &label)?;
        } else if extension(&path).as_deref() == Some("one") {
            jobs.push(Job {
                folders: folders.to_vec(),
                stem: stem(&path),
                file: path,
                label,
            });
            if jobs.len() > MAX_SECTIONS {
                return Err("the export holds more than 500 sections".into());
            }
        }
    }
    Ok(())
}

/// An unpacked archive's notebook: a single top folder is the notebook itself; otherwise the
/// archive's own name is.
fn unpacked(root: &Path, archive_name: String, jobs: &mut Vec<Job>) -> Result<()> {
    let top: Vec<_> = fs::read_dir(root)?.collect::<io::Result<Vec<_>>>()?;
    if let [only] = top.as_slice()
        && only.file_type()?.is_dir()
    {
        let name = only.file_name().to_string_lossy().into_owned();
        return walk(&only.path(), std::slice::from_ref(&name), jobs, &name);
    }
    walk(root, &[archive_name], jobs, "")
}

/// The sections in `source`. Archives unpack below `scratch`, which must exist.
pub fn inventory(source: &Path, scratch: &Path) -> Result<Vec<Job>> {
    let mut jobs = Vec::new();
    match extension(source).as_deref() {
        Some("one") => {
            let name = source
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            jobs.push(Job {
                folders: Vec::new(),
                stem: stem(source),
                file: source.to_path_buf(),
                label: name,
            });
        }
        Some("onetoc2") => {
            let root = source.parent().ok_or("the notebook file has no folder")?;
            let name = root
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| "OneNote".into());
            walk(root, std::slice::from_ref(&name), &mut jobs, &name)?;
        }
        Some("onepkg") => {
            let into = scratch.join("package");
            extract_cab(source, &into)?;
            unpacked(&into, stem(source), &mut jobs)?;
        }
        Some("zip") => {
            let into = scratch.join("zip");
            extract_zip(source, &into)?;
            unpacked(&into, stem(source), &mut jobs)?;
        }
        _ => {
            return Err(
                "choose a OneNote section (.one), notebook (.onetoc2), package (.onepkg) or ZIP export"
                    .into(),
            );
        }
    }
    if jobs.is_empty() {
        return Err("no OneNote sections (.one files) were found".into());
    }
    Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    fn zip_file(path: &Path, entries: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        fs::write(path, writer.finish().unwrap().into_inner()).unwrap();
    }

    #[test]
    fn web_export_zip_keeps_groups_and_skips_everything_else() {
        let home = tempfile::tempdir().unwrap();
        let archive = home.path().join("Physics.zip");
        zip_file(
            &archive,
            &[
                ("Physics/Open Notebook.onetoc2", b"toc"),
                ("Physics/Mechanics.one", b"a"),
                ("Physics/Lab/Week 1.one", b"b"),
                ("Physics/OneNote_RecycleBin/OneNote_DeletedPages.one", b"c"),
                ("Physics/readme.exe", b"d"),
            ],
        );
        let scratch = home.path().join("scratch");
        fs::create_dir(&scratch).unwrap();
        let jobs = inventory(&archive, &scratch).unwrap();
        let summary: Vec<_> = jobs
            .iter()
            .map(|job| (job.folders.join("/"), job.stem.as_str(), job.label.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                ("Physics/Lab".to_owned(), "Week 1", "Physics/Lab/Week 1.one"),
                ("Physics".to_owned(), "Mechanics", "Physics/Mechanics.one"),
            ]
        );
        assert!(!scratch.join("zip/Physics/readme.exe").exists());
        assert!(!scratch.join("zip/Physics/Open Notebook.onetoc2").exists());
    }

    #[test]
    fn loose_sections_use_the_archive_name() {
        let home = tempfile::tempdir().unwrap();
        let archive = home.path().join("Export.zip");
        zip_file(&archive, &[("A.one", b"a"), ("B.one", b"b")]);
        let jobs = inventory(&archive, home.path()).unwrap();
        assert!(jobs.iter().all(|job| job.folders == ["Export"]));
    }

    #[test]
    fn unsafe_duplicate_and_empty_archives_are_refused() {
        let home = tempfile::tempdir().unwrap();
        for (index, entries) in [
            vec![("../evil.one", b"x".as_slice())],
            vec![("a/C:evil.one", b"x".as_slice())],
            vec![("a/CON.one", b"x".as_slice())],
            vec![("a/b.one", b"x".as_slice()), ("A/B.ONE", b"y".as_slice())],
            vec![("notes.txt", b"x".as_slice())],
        ]
        .into_iter()
        .enumerate()
        {
            let archive = home.path().join(format!("case{index}.zip"));
            zip_file(&archive, &entries);
            let scratch = home.path().join(format!("scratch{index}"));
            fs::create_dir(&scratch).unwrap();
            assert!(inventory(&archive, &scratch).is_err(), "{entries:?}");
        }
    }

    #[test]
    fn notebook_folder_through_its_table_of_contents() {
        let home = tempfile::tempdir().unwrap();
        let notebook = home.path().join("Math");
        fs::create_dir_all(notebook.join("Group")).unwrap();
        fs::write(notebook.join("Open Notebook.onetoc2"), b"toc").unwrap();
        fs::write(notebook.join("Algebra.one"), b"a").unwrap();
        fs::write(notebook.join("Group/Proofs.one"), b"b").unwrap();
        let jobs = inventory(&notebook.join("Open Notebook.onetoc2"), home.path()).unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].folders, ["Math"]);
        assert_eq!(jobs[1].folders, ["Math", "Group"]);
        assert!(inventory(&home.path().join("x.pdf"), home.path()).is_err());
    }
}
