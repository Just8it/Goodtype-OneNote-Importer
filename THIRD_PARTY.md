# Third-party components

- [onenote_parser](https://github.com/msiemens/onenote.rs) 2.1.0 (upstream commit
  `727389a674665dc07e0c31818cd4b54b64b9c684`), MPL-2.0. Decodes OneNote section files. Used
  unmodified from crates.io; its source is available at the link above. Its own dependencies
  are listed in `licenses/rust-dependencies.json`. Its MPL-2.0 text, also covering
  `onenote_parser-macros`, is retained from that upstream revision in
  `licenses/onenote.rs-MPL-2.0.txt` because the crates.io archives omit the license file.
- [goodtype-core](https://github.com/Just8it/Goodtype) at a pinned revision: the canonical
  notebook writer and validator, shared with Goodtype itself.
- Direct Rust dependencies, each with one responsibility: `cab` and `zip` unpack `.onepkg`
  packages and ZIP exports into bounded scratch folders; `image` (PNG, JPEG, GIF, BMP, TIFF)
  decodes pictures with dimension and allocation limits and re-encodes non-PNG/JPEG formats;
  `serde`/`serde_json` write reports; `tempfile` stages work; `sha2` fingerprints originals;
  `uuid` supplies notebook identities; `time` records UTC timestamps; `typed-path` names the
  section for the decoder.

`licenses/rust/` in the release package contains every redistributed Rust dependency's license
and notice files; `licenses/rust-dependencies.json` records versions and SPDX expressions.

OneNote is a trademark of Microsoft. This project is not affiliated with or endorsed by Microsoft.
