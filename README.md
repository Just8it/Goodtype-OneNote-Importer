# Goodtype OneNote Importer

Converts OneNote notebooks and sections into ordinary [Goodtype](https://github.com/Just8it/Goodtype)
notebooks, locally. Goodtype downloads this tool only when you choose **New → Import… → OneNote**
in its Library; Goodtype and imported notebooks work without it.

Status: alpha. It converts the onenote.rs sample corpus (OneNote 2016, OneNote for Windows,
OneDrive and OneNote for the web files); real notebooks may contain content it does not yet
carry over, and each such case is reported.

## What you can import

| Export | How to get it |
|---|---|
| A section (`.one`) | OneNote for Windows: **File → Export → Section → OneNote Section (*.one)** |
| A notebook package (`.onepkg`) | OneNote for Windows: **File → Export → Notebook → OneNote Package (*.onepkg)** |
| A notebook folder (`.onetoc2`) | Pick the `Open Notebook.onetoc2` inside a local notebook folder |
| A web export (`.zip`) | OneNote for the web (personal OneDrive): **Notebooks → Export notebook** |

Each section becomes one Goodtype notebook. The OneNote notebook and its section groups become
Library folders. Every notebook keeps the section's original file as `originals/source.one`.

## What carries over

- **Typed text** becomes editable Typst blocks: bold, italic, underline, strikethrough,
  super/subscript, sizes, colours, highlights, web links, alignment, indentation, bullet and
  numbered lists, to-do boxes and tables.
- **Equations** become Typst math (fractions, roots, sub/superscripts, integrals and sums with
  limits, brackets, matrices, functions). An equation that cannot be converted stays as text.
- **Handwriting** becomes editable Goodtype ink with its colour, width and transparency.
  OneNote does not store pen pressure, so strokes have constant width.
- **Pictures** become images (PNG/JPEG kept; GIF, BMP and TIFF converted to PNG; EMF/WMF use
  OneNote's own web copy when present). Pictures inside text stay inside their Typst block.
- **PDF printouts** come through as their page pictures; the attached PDF is kept.
- **Attachments** are kept under `originals/attachments/`, marked where they sat.

OneNote pages are an open canvas. A tall page becomes several A4 pages, cut only between strokes,
pictures and paragraphs, preferring wide empty bands; a page grows past A4 only where one piece
(a large picture or drawing) leaves no clean cut.

## Not carried over

Audio and video playback, OneNote-internal page links (kept as text), note tags other than
to-do boxes, handwriting inside table cells, and ink recognition. Handwriting that sits inside a
text container is placed next to its text approximately. Each notebook's `import-report.json`
lists what happened per page.

## Command line

```text
goodtype-onenote-import <section.one|Open Notebook.onetoc2|package.onepkg|export.zip> <NEW folder>
goodtype-onenote-import --version --json
goodtype-onenote-import --check
```

The destination must not exist. Results are published notebook by notebook into it, with an
`import-report.json` listing each section. See [INTEGRATION.md](INTEGRATION.md) for the
contract Goodtype relies on and [RELEASING.md](RELEASING.md) for releases.

## Development

```text
cargo xtask verify
ONENOTE_SAMPLES=<folder with .one files> cargo test -- --ignored
```

OneNote files are never committed. License: Apache-2.0; see [THIRD_PARTY.md](THIRD_PARTY.md).
