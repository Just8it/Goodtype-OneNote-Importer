"""Build a portable Windows x64 release. Everything is compiled in; nothing is downloaded at runtime."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
NAME = "goodtype-onenote-import"
CORE_REVISION = "01aefd9fec51ee4d06e85c7b0a48285d6ba90c0c"
# Upstream onenote.rs commit tagged v2.1.0, the exact onenote_parser release in Cargo.lock.
DECODER_REVISION = "727389a674665dc07e0c31818cd4b54b64b9c684"
REPOSITORY = "https://github.com/Just8it/Goodtype-OneNote-Importer"


def run(*args):
    subprocess.run(args, cwd=ROOT, check=True)


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def collect_rust_licenses(destination):
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", "x86_64-pc-windows-msvc"], cwd=ROOT))
    catalog = []
    for package in metadata["packages"]:
        if package["name"] == NAME:
            continue
        source = Path(package["manifest_path"]).parent
        files = []
        for pattern in ("LICENSE*", "LICENCE*", "COPYING*", "NOTICE*"):
            files.extend(path for path in source.glob(pattern) if path.is_file())
        if not files and package["name"] == "goodtype-core":
            # Goodtype keeps its workspace license and notice above crates/goodtype-core.
            files = [source.parents[1]/"LICENSE", source.parents[1]/"NOTICE"]
        if package.get("license_file"):
            files.append(source/package["license_file"])
        if not files and package["name"] in ("onenote_parser", "onenote_parser-macros"):
            # These published crates omit the license held at their pinned upstream root.
            files = [ROOT/"licenses/onenote.rs-MPL-2.0.txt"]
        if not files:
            raise ValueError(f"no distributable license found for {package['name']}")
        target = destination/"licenses/rust"/f"{package['name']}-{package['version']}"
        target.mkdir(parents=True)
        for path in sorted(set(files)):
            shutil.copy2(path, target/path.name)
        catalog.append({"name": package["name"], "version": package["version"],
                        "license": package.get("license"), "source": package.get("source")})
    (destination/"licenses/rust-dependencies.json").write_text(json.dumps(catalog, indent=2)+"\n", encoding="utf8")


def zip_bundle(staging, archive_path):
    # Upstream license files may use epoch timestamps, outside ZIP's 1980–2107 range.
    with zipfile.ZipFile(archive_path, "x", zipfile.ZIP_DEFLATED, strict_timestamps=False) as archive:
        for path in sorted(staging.rglob("*")):
            if path.is_file():
                archive.write(path, (Path(staging.name)/path.relative_to(staging)).as_posix())


def publish_folder(staging, destination):
    # Windows scanners can briefly retain a handle after the self-check exits.
    for attempt in range(20):
        try:
            staging.rename(destination)
            return
        except PermissionError:
            if attempt == 19:
                raise
            time.sleep(.25)


def build(output):
    if sys.platform != "win32" or platform.machine().lower() not in ("amd64", "x86_64"):
        raise ValueError("build the portable release on a Windows x64 host")
    version = tomllib.loads((ROOT/"Cargo.toml").read_text(encoding="utf8"))["package"]["version"]
    name = f"goodtype-onenote-importer-{version}-windows-x64"
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    bundle = output/name
    archive_path = output/(name+".zip")
    descriptor_path = output/(name+".json")
    if any(p.exists() for p in (bundle, archive_path, descriptor_path)):
        raise ValueError("release output already exists; choose a fresh --output directory")
    run("cargo", "build", "--release", "--locked", "--bin", NAME)
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip())
    with tempfile.TemporaryDirectory(prefix=".package-", dir=output) as temporary:
        staging = Path(temporary)/name
        staging.mkdir()
        for source, target in [(ROOT/"target/release"/f"{NAME}.exe", f"{NAME}.exe"),
                               (ROOT/"README.md", "README.md"), (ROOT/"LICENSE", "LICENSE"),
                               (ROOT/"THIRD_PARTY.md", "THIRD_PARTY.md")]:
            shutil.copy2(source, staging/target)
        collect_rust_licenses(staging)
        metadata = {"version": version, "protocolVersion": 1, "notebookSchemaVersion": 3,
                    "target": "x86_64-pc-windows-msvc", "sourceCommit": commit, "sourceDirty": dirty,
                    "goodtypeCoreRevision": CORE_REVISION, "decoderCommit": DECODER_REVISION}
        metadata["files"] = [{"path": p.relative_to(staging).as_posix(), "sizeBytes": p.stat().st_size,
                              "sha256": sha256(p)} for p in sorted(staging.rglob("*")) if p.is_file()]
        (staging/"importer-manifest.json").write_text(json.dumps(metadata, indent=2)+"\n", encoding="utf8")
        run(str(staging/f"{NAME}.exe"), "--check")
        staged_archive = Path(temporary)/archive_path.name
        zip_bundle(staging, staged_archive)
        publish_folder(staging, bundle)
        staged_archive.rename(archive_path)
    descriptor = {k: v for k, v in metadata.items() if k != "files"}
    descriptor["archive"] = {"fileName": archive_path.name, "sizeBytes": archive_path.stat().st_size,
                             "sha256": sha256(archive_path),
                             "url": f"{REPOSITORY}/releases/download/v{version}/{archive_path.name}"}
    descriptor_path.write_text(json.dumps(descriptor, indent=2)+"\n", encoding="utf8")
    (output/"SHA256SUMS").write_text(f"{sha256(archive_path)}  {archive_path.name}\n{sha256(descriptor_path)}  {descriptor_path.name}\n", encoding="utf8")
    print(f"Portable release: {archive_path}")
    return archive_path


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT/"dist")
    build(parser.parse_args().output)
