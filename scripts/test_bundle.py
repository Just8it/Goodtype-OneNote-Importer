"""Check a relocated release with no development tools on PATH and no private notebook data."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile


def test_bundle(archive_path):
    with tempfile.TemporaryDirectory(prefix="goodtype portable test ") as directory:
        root = Path(directory)
        with zipfile.ZipFile(archive_path) as archive:
            archive.extractall(root)
        bundle, = root.glob("goodtype-onenote-importer-*")
        manifest = json.loads((bundle/"importer-manifest.json").read_text())
        expected = {"importer-manifest.json"}
        for entry in manifest["files"]:
            path = bundle/entry["path"]
            assert path.stat().st_size == entry["sizeBytes"]
            assert hashlib.sha256(path.read_bytes()).hexdigest() == entry["sha256"]
            expected.add(entry["path"])
        assert {p.relative_to(bundle).as_posix() for p in bundle.rglob("*") if p.is_file()} == expected
        environment = os.environ.copy()
        environment["PATH"] = str(Path(os.environ["SystemRoot"])/"System32")
        binary = bundle/"goodtype-onenote-import.exe"

        def command(*args, success=True):
            result = subprocess.run([str(binary), *map(str, args)], cwd=root, env=environment,
                                    capture_output=True, text=True, timeout=60)
            assert result.returncode == (0 if success else 1), result.stderr
            return result.stdout

        info = json.loads(command("--version", "--json"))
        assert info["name"] == "goodtype-onenote-import"
        for key in ("version", "protocolVersion", "notebookSchemaVersion", "decoderCommit"):
            assert info[key] == manifest[key]
        assert info["protocolVersion"] == 1 and info["notebookSchemaVersion"] == 3
        assert info["decoderCommit"] in command("--check")
        command("--help")

        # Exercise the compiled decoder and batch report with deliberately invalid sections.
        source = root/"Sections.zip"
        with zipfile.ZipFile(source, "w") as archive:
            archive.writestr("Notebook/Lectures/First.one", b"invalid OneNote fixture")
            archive.writestr("Notebook/Second.one", b"another invalid section")
        original = source.read_bytes()
        destination = root/"Imported notebooks"
        command(source, destination, success=False)
        report_path = destination/"import-report.json"
        report = json.loads(report_path.read_text())
        assert len(report) == 2
        assert all(entry["status"] == "failed" and entry["error"] for entry in report)
        assert not list(destination.rglob("goodtype.json"))
        before = report_path.read_bytes()
        command(source, destination, success=False)
        assert report_path.read_bytes() == before
        assert source.read_bytes() == original
        assert not list(root.glob(".onenote-import-*"))
        print("Portable smoke passed: hashes, isolated PATH, contract, decoder failures, batch continuation, source retention and overwrite refusal.")


if __name__ == "__main__":
    test_bundle(Path(sys.argv[1]).resolve())
