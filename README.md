[![status](https://img.shields.io/badge/status-alpha-red)](#alpha)
[![license](https://img.shields.io/badge/license-Apache%202.0-green)](./LICENSE)
[![platform](https://img.shields.io/badge/platform-Windows-blue)](#use)
[![built with](https://img.shields.io/badge/built%20with-Rust-orange)](#build)

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/mark-dark.svg">
    <img src="brand/mark-light.svg" width="130" alt="Goodtype">
  </picture>
</p>

<h1 align="center">Goodtype OneNote Importer</h1>

<p align="center">Bring your OneNote notebooks into Goodtype. Keep your notes on your own disk.</p>

Convert a OneNote section, notebook folder, package or web export into ordinary
[Goodtype](https://github.com/Just8it/Goodtype) notebooks. Each section becomes a notebook;
notebook folders and section groups keep their structure in the Library.

Conversion works offline. No account, no upload, no telemetry. Goodtype works without this tool,
and imported notebooks stay usable if you remove it.

## Alpha

> [!WARNING]
> **The OneNote importer is experimental. Keep your original exports and review the result.**
> Tested against the onenote.rs sample corpus; other notebooks may contain unsupported content.
> Audio/video playback, internal page links, most note tags and ink inside table cells are not
> carried over. Handwriting inside text containers is positioned approximately.

- **Text** becomes editable Typst blocks, including formatting, lists, to-do boxes, tables and links.
- **Equations** become Typst math; unsupported equations stay as text.
- **Handwriting** becomes editable ink with its colour, width and transparency, at constant width.
- **Pictures and PDF printouts** become images. Attachments are preserved as original files.

Tall pages split between strokes, pictures and paragraphs, preferring empty bands. A page grows
past A4 where a single piece of content leaves no clean cut. Each notebook keeps its original
section in `originals/source.one` and a report of page counts and warnings. Existing destinations
are never replaced.

## Use

Download the **Windows x64 ZIP** from [Releases](https://github.com/Just8it/Goodtype-OneNote-Importer/releases),
extract it and keep the whole folder together. No Python or separate decoder installation is needed.

```powershell
.\goodtype-onenote-import.exe 'C:\Notes\Notebook.onepkg' 'C:\My Library\Imported OneNote'
```

The destination must be a new folder. `--help` lists the options; `--check` verifies the executable.
Open the converted notebooks in Goodtype. In-app download/import integration is a separate step.

| Source | How to get it |
|---|---|
| Section (`.one`) | OneNote for Windows: **File → Export → Section → OneNote Section** |
| Package (`.onepkg`) | OneNote for Windows: **File → Export → Notebook → OneNote Package** |
| Local notebook (`.onetoc2`) | Pick `Open Notebook.onetoc2` inside the notebook folder |
| Web export (`.zip`) | OneNote for the web, personal OneDrive: **Notebooks → Export notebook** |

## Build

Requires Python 3.11+ for build checks, the pinned Rust toolchain, and Visual Studio C++ Build Tools
on Windows. Python is used only during development; the released converter is a single executable.

```powershell
cargo xtask verify
python scripts/build_windows.py
python scripts/test_bundle.py dist/goodtype-onenote-importer-0.1.0-windows-x64.zip
```

The portable folder, ZIP and checksums appear in `dist/`. Dependencies download during setup/build
only. [Release integration](./INTEGRATION.md) describes the version contract.
[Development and releases](./RELEASING.md) explains `dev`/`main` promotion and automatic signed releases.
Real-file checks are opt-in: set `ONENOTE_SAMPLES` and run `cargo test -- --ignored`.

## License

Apache License 2.0 — see [LICENSE](./LICENSE). The decoder is MPL-2.0 licensed;
[THIRD_PARTY.md](./THIRD_PARTY.md) records the pinned components. Release bundles include dependency
licenses. This project is not affiliated with or endorsed by Microsoft.
