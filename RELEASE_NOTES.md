# Goodtype OneNote Importer 0.1.0

First release. Converts OneNote sections (`.one`), notebook folders (`.onetoc2`), packages
(`.onepkg`) and OneNote-for-the-web ZIP exports into ordinary Goodtype notebooks.

- Typed text, lists, to-do boxes, tables and links become editable Typst blocks.
- Equations become Typst math.
- Handwriting becomes editable ink (constant width; OneNote stores no pen pressure).
- Pictures and PDF printouts become images; attachments are kept with the notebook.
- Tall OneNote pages are split into A4 pages between strokes, pictures and paragraphs.

Alpha: verified against the onenote.rs sample corpus. Please report notebooks that do not
convert as expected; never attach private notebooks to public issues.
