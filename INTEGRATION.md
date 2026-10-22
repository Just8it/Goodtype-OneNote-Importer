# Release contract

This tool follows the release contract used by Goodtype's Goodnotes importer (Goodtype
ADR 0029/0031). OneNote desktop integration is separate work: it must pin a tested package
and the OneNote public key before offering downloads or updates. This file is the contract.

## Package

`goodtype-onenote-importer-<version>-windows-x64.zip` contains one folder of the same name:

```text
goodtype-onenote-import.exe
importer-manifest.json    # versions, source revisions and the size/SHA-256 of every file
licenses/                 # Rust dependency licenses
LICENSE
README.md
THIRD_PARTY.md
```

There is no runtime, interpreter or helper process: the decoder (`onenote_parser`) and the
canonical writer (`goodtype-core`) are compiled into the executable. The sibling release JSON
describes target, protocol/schema versions, archive URL, size and SHA-256. A checksum downloaded
beside an untrusted archive is not a trust anchor: Goodtype pins the tested archive hash in its
own source, and accepts updates only through signed metadata (below).

## Compatibility

`goodtype-onenote-import.exe --version --json` returns:

```json
{"name":"goodtype-onenote-import","version":"0.1.0","protocolVersion":1,"notebookSchemaVersion":3,"decoderCommit":"727389a674665dc07e0c31818cd4b54b64b9c684"}
```

`decoderCommit` is the upstream onenote.rs commit tagged `v2.1.0`, the exact `onenote_parser`
release locked in `Cargo.lock`. `--check` proves the executable starts and links its decoder and
writer; its output is human-readable text. Exit 0 means success; any failure exits 1.

## Execution

```text
goodtype-onenote-import <source> <NEW batch folder> [--allow-derived] [--title <title>]
```

`--allow-derived` is accepted for protocol compatibility and changes nothing: OneNote content is
never turned into vector artwork. The batch folder's `import-report.json` starts with `pending`
entries and records `completed`/`failed`, source, target, page count and error per section, as
the Goodnotes importer does. Each completed notebook has its own `import-report.json` with page
notes and `originals/source.one`. Human-readable stdout is not a machine event stream. Imports
exit 1 when any section failed, keeping every completed notebook.

A section that fails to decode (including a decoder panic) fails alone; the batch continues.
Inputs are bounded: 512 MiB per section file and per unpacked archive, 10,000 archive entries,
500 sections, 12 section-group levels, 2,000 pages per notebook. Archives unpack only `.one`
files with portable relative names, no links, no duplicates (case-insensitive), counted while
writing. Pictures pass a bounded full decode. Goodtype runs the tool inside a Windows Job Object
with memory and time limits; it is not a filesystem sandbox.

## Updates

An explicit update check fetches the fixed latest-release `importer-update.json` and its
`importer-update.json.sig` from that exact version. The signature is Ed25519 over the exact bytes,
verified against a public key compiled into Goodtype. The publisher signs only in the protected
`main` release job, with the private key held as the `IMPORTER_SIGNING_KEY` repository secret.
The public half is `release-public-key.pem`; it is distinct from the Goodnotes signing key.
Goodtype must pin it before offering OneNote updates. Until desktop integration is implemented,
users download versioned releases directly.

## Maintenance

`goodtype-core` comes from a pinned Goodtype Git revision; upgrade it deliberately and re-run
the opt-in sample conversion. Never copy or fork its persistence code. Upgrade `onenote_parser`
the same way and update `DECODER_PIN` in `src/lib.rs` and `scripts/build_windows.py` together.
