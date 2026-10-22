//! Opt-in conversion of real OneNote sections. OneNote files are personal or third-party
//! material and never live in this repository; point `ONENOTE_SAMPLES` at a folder of `.one`
//! files (for example onenote.rs's `crates/parser/tests/samples`) and run
//! `cargo test -- --ignored`.

use std::{fs, path::Path};

use goodtype_onenote_import::{MAX_SECTION_BYTES, convert, read_limited, read_section};

fn sections(folder: &Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(folder).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sections(&path, found);
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("one"))
        {
            found.push(path);
        }
    }
}

#[test]
#[ignore = "needs real OneNote sections in ONENOTE_SAMPLES"]
fn real_sections_convert_into_notebooks_that_reopen() {
    let root = std::path::PathBuf::from(std::env::var_os("ONENOTE_SAMPLES").unwrap());
    let mut found = Vec::new();
    sections(&root, &mut found);
    assert!(!found.is_empty(), "no .one files below {}", root.display());
    let out = tempfile::tempdir().unwrap();
    let mut converted = 0;
    for (index, path) in found.iter().enumerate() {
        let bytes = read_limited(path, MAX_SECTION_BYTES).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // Sections OneNote itself cannot open (for example deleted-page bins) may fail; they
        // must fail as errors, never as panics or half-written notebooks.
        let Ok((title, pages)) = read_section(&bytes, &name) else {
            println!("skipped {name}: not readable as a section");
            continue;
        };
        let destination = out.path().join(format!("notebook-{index}"));
        let report = convert(pages, &bytes, &destination, &title, Vec::new())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let pages: Vec<_> = report.pages.iter().map(|page| page.page).collect();
        assert_eq!(
            pages,
            (1..=report.pages.len()).collect::<Vec<_>>(),
            "{name}"
        );
        assert!(destination.join("originals/source.one").is_file());
        converted += 1;
        println!("{name}: {} pages", report.pages.len());
    }
    assert!(converted > 0);
}
