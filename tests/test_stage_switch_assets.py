#!/usr/bin/env python3
"""Tests for the offline Switch asset importer (tools/stage_switch_assets.py).

Synthetic client jars only: no Mojang file is read, copied or committed. The
internal `Pin` lets a test bind a fixture jar; the CLI keeps its own constants.
"""
import contextlib
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
import warnings
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import stage_switch_assets as staging  # noqa: E402

MODEL_PARENT = b'{"parent":"minecraft:block/cube","textures":{"all":"minecraft:block/stone"}}'
BLOCK_MODEL = (b'{"parent":"minecraft:block/cube_all","textures":{"all":'
               b'{"sprite":"block/stone","force_translucent":true}}}')
EMERALD_MODEL = b'{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:item/emerald.png"}}'
GHOST_MODEL = b'{"parent":"minecraft:block/cube_all","textures":{"all":"item/absent_texture"}}'
VARIABLE_MODEL = b'{"parent":"minecraft:block/cube_all","textures":{"all":"#other"}}'
BLOCKSTATE = b'{"variants":{"":{"model":"minecraft:block/stone"}}}'
TEXTURE = b"\x89PNG\r\n\x1a\nsynthetic-stone-texture"
MCMETA = b'{"animation":{"frametime":2}}'
COLORMAP = b"\x89PNG\r\n\x1a\nsynthetic-grass-colormap"
EXTERNAL = b"\x89PNG\r\n\x1a\nsynthetic-emerald-item-texture"
EXTERNAL_MCMETA = b'{"animation":{"frametime":3}}'

BASE = {
    "assets/minecraft/models/block/cube.json": MODEL_PARENT,
    "assets/minecraft/models/block/stone.json": BLOCK_MODEL,
    "assets/minecraft/models/block/emerald_block.json": EMERALD_MODEL,
    "assets/minecraft/models/block/ghost_block.json": GHOST_MODEL,
    "assets/minecraft/models/block/variable.json": VARIABLE_MODEL,
    "assets/minecraft/blockstates/stone.json": BLOCKSTATE,
    "assets/minecraft/textures/block/stone.png": TEXTURE,
    "assets/minecraft/textures/block/stone.png.mcmeta": MCMETA,
    "assets/minecraft/textures/colormap/grass.png": COLORMAP,
}
REFERENCED = {
    "assets/minecraft/textures/item/emerald.png": EXTERNAL,
    "assets/minecraft/textures/item/emerald.png.mcmeta": EXTERNAL_MCMETA,
}
EXPECTED_FILES = dict(BASE, **REFERENCED)
UNSELECTED = {
    "assets/minecraft/models/block/notes.txt": b"not selected: json only",
    "assets/minecraft/textures/item/apple.png": b"not selected: unreferenced",
    "assets/minecraft/sounds/random/click.ogg": b"not selected: audio",
}
MISSING = [{
    "model": "assets/minecraft/models/block/ghost_block.json",
    "texture": "item/absent_texture",
    "expected": "assets/minecraft/textures/item/absent_texture.png",
}]


def make_jar(path, entries=None, version=None):
    """Write a synthetic client jar and return the Pin that accepts it."""
    entries = dict(EXPECTED_FILES if entries is None else entries)
    version = ({"id": staging.PINNED_MINECRAFT,
                "protocol_version": staging.PINNED_PROTOCOL}
               if version is None else version)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_STORED) as archive:
        archive.writestr("version.json", json.dumps(version))
        for name, data in entries.items():
            archive.writestr(name, data)
    digest = hashlib.sha1(Path(path).read_bytes()).hexdigest()
    return staging.Pin(staging.PINNED_MINECRAFT, staging.PINNED_PROTOCOL, digest)


def pin_for(path):
    return staging.Pin(staging.PINNED_MINECRAFT, staging.PINNED_PROTOCOL,
                       hashlib.sha1(Path(path).read_bytes()).hexdigest())


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def expected_manifest(jar, files):
    return {
        "schema": 1,
        "minecraft": staging.PINNED_MINECRAFT,
        "protocol": staging.PINNED_PROTOCOL,
        "source_sha1": hashlib.sha1(Path(jar).read_bytes()).hexdigest(),
        "count": len(files),
        "total_bytes": sum(len(data) for data in files.values()),
        "texture_references": {
            "models_scanned": 5,
            "texture_bindings": 4,
            "extra_files": sorted(REFERENCED),
            "missing": MISSING,
        },
        "files": [{"path": name, "size": len(files[name]),
                   "sha256": sha256(files[name])}
                  for name in sorted(files)],
    }


class StagingTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.jar = self.root / "client.jar"
        self.output = self.root / "stage" / "26.3"

    def tearDown(self):
        self._tmp.cleanup()

    def leftovers(self):
        return sorted(path.name for path in self.root.iterdir())

    def test_extracts_selection_and_writes_matching_manifest(self):
        pin = make_jar(self.jar, dict(EXPECTED_FILES, **UNSELECTED))
        result = staging.stage(self.jar, self.output, pin=pin)

        self.assertFalse(result.dry_run)
        self.assertFalse(result.verified_existing)
        staged = sorted(path.relative_to(self.output).as_posix()
                        for path in self.output.rglob("*") if path.is_file())
        self.assertEqual(staged, sorted([*EXPECTED_FILES, "manifest.json"]))
        for name, data in EXPECTED_FILES.items():
            self.assertEqual((self.output / name).read_bytes(), data)

        manifest = json.loads((self.output / "manifest.json").read_text())
        self.assertEqual(manifest, expected_manifest(self.jar, EXPECTED_FILES))
        self.assertEqual(result.manifest, manifest)
        self.assertEqual(result.count, len(EXPECTED_FILES))
        self.assertFalse((self.output / "assets/minecraft/textures/item/apple.png").exists())

    def test_dry_run_reports_without_extracting(self):
        pin = make_jar(self.jar)
        result = staging.stage(self.jar, self.output, pin=pin, dry_run=True)
        self.assertTrue(result.dry_run)
        self.assertFalse(result.verified_existing)
        self.assertIsNone(result.manifest)
        self.assertEqual(result.count, len(EXPECTED_FILES))
        self.assertEqual(result.total_bytes,
                         sum(len(data) for data in EXPECTED_FILES.values()))
        self.assertEqual(result.source_sha1, pin.client_sha1)
        self.assertEqual(result.references["missing"], MISSING)
        stream = io.StringIO()
        with contextlib.redirect_stdout(stream):
            staging._report_references(result.references)
        text = stream.getvalue()
        self.assertIn("block models scanned: 5", text)
        self.assertIn("missing texture references: 1", text)
        self.assertIn("item/absent_texture", text)
        self.assertIn("referenced: assets/minecraft/textures/item/emerald.png",
                      text)
        self.assertFalse(self.output.exists())
        self.assertFalse(self.output.parent.exists())
        self.assertEqual(self.leftovers(), ["client.jar"])

    def test_reports_missing_reference_without_synthesizing_it(self):
        entries = {name: data for name, data in EXPECTED_FILES.items()
                   if "emerald" not in name}
        pin = make_jar(self.jar, entries)
        result = staging.stage(self.jar, self.output, pin=pin)
        self.assertEqual(result.references["missing"], MISSING)
        self.assertEqual(result.references["extra_files"], [])
        self.assertFalse(
            (self.output / "assets/minecraft/textures/item/absent_texture.png")
            .exists())
        manifest = json.loads((self.output / "manifest.json").read_text())
        self.assertEqual(manifest["texture_references"]["missing"], MISSING)

    def test_pin_mismatch_leaves_no_output(self):
        pin = make_jar(self.jar)
        wrong = [
            staging.Pin(pin.minecraft, pin.protocol, "0" * 40),
            staging.Pin("26.2", pin.protocol, pin.client_sha1),
            staging.Pin(pin.minecraft, 776, pin.client_sha1),
        ]
        for candidate in wrong:
            with self.subTest(pin=candidate):
                with self.assertRaises(staging.StageError):
                    staging.stage(self.jar, self.output, pin=candidate)
                self.assertFalse(self.output.exists())
                self.assertEqual(self.leftovers(), ["client.jar"])

        # The CLI pins the real jar hash and offers no flag to replace it.
        cli_cases = [
            (["--client-jar", str(self.jar), "--output", str(self.output)], 1),
            (["--client-jar", str(self.jar), "--output", str(self.output),
              "--client-sha1", "0" * 40], 2),
        ]
        for argv, expected in cli_cases:
            with self.subTest(argv=argv[-1]):
                with contextlib.redirect_stderr(io.StringIO()):
                    if expected == 2:
                        with self.assertRaises(SystemExit) as caught:
                            staging.main(argv)
                        self.assertEqual(caught.exception.code, 2)
                    else:
                        self.assertEqual(staging.main(argv), 1)
                self.assertFalse(self.output.exists())

        lock = self.root / "research-lock.json"
        lock.write_text(json.dumps({"minecraft": pin.minecraft,
                                    "protocol": pin.protocol,
                                    "client_sha1": "0" * 40}))
        with self.assertRaises(staging.StageError):
            staging.stage(self.jar, self.output,
                          pin=staging.Pin.from_research_lock(lock))
        self.assertFalse(self.output.exists())

        matching = self.root / "matching-lock.json"
        matching.write_text(json.dumps({
            "minecraft": staging.PINNED_MINECRAFT,
            "protocol": staging.PINNED_PROTOCOL,
            "client_sha1": staging.PINNED_CLIENT_SHA1}))
        self.assertEqual(staging.Pin.from_research_lock(matching),
                         staging.Pin.pinned())

        matching.write_text(json.dumps({"minecraft": {
            "version": staging.PINNED_MINECRAFT,
            "client_sha1": staging.PINNED_CLIENT_SHA1}}))
        self.assertEqual(staging.Pin.from_research_lock(matching),
                         staging.Pin.pinned())
        matching.write_text(json.dumps({"minecraft": {
            "version": staging.PINNED_MINECRAFT,
            "client_sha1": "0" * 40}}))
        with self.assertRaises(staging.StageError):
            staging.Pin.from_research_lock(matching)

        fixture_lock = self.root / "fixture-lock.json"
        fixture_lock.write_text(json.dumps({"minecraft": pin.minecraft,
                                            "protocol": pin.protocol,
                                            "client_sha1": pin.client_sha1}))
        with self.assertRaises(staging.StageError):
            staging.Pin.from_research_lock(fixture_lock)

    def test_rejects_traversal_absolute_duplicate_and_symlink_entries(self):
        cases = {
            "parent traversal": {"assets/minecraft/models/../../escape.json": b"{}"},
            "inner traversal": {"assets/minecraft/models/../textures/block/x.png": b"x"},
            "absolute": {"/assets/minecraft/models/x.json": b"{}"},
            "backslash": {"assets\\minecraft\\models\\x.json": b"{}"},
            "empty component": {"assets/minecraft/models//x.json": b"{}"},
        }
        for label, entries in cases.items():
            with self.subTest(case=label):
                staging_root = self.root / f"case-{abs(hash(label))}"
                jar = staging_root / "client.jar"
                output = staging_root / "out"
                staging_root.mkdir()
                pin = make_jar(jar, entries)
                with self.assertRaises(staging.StageError):
                    staging.stage(jar, output, pin=pin)
                self.assertFalse(output.exists())
                self.assertEqual(sorted(p.name for p in staging_root.iterdir()),
                                 ["client.jar"])

        with self.subTest(case="duplicate"):
            jar = self.root / "dup.jar"
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", UserWarning)
                with zipfile.ZipFile(jar, "w", zipfile.ZIP_STORED) as archive:
                    archive.writestr("version.json", json.dumps(
                        {"id": staging.PINNED_MINECRAFT,
                         "protocol_version": staging.PINNED_PROTOCOL}))
                    archive.writestr("assets/minecraft/models/a.json", b"{}")
                    archive.writestr("assets/minecraft/models/a.json", b"{}")
            with self.assertRaises(staging.StageError):
                staging.stage(jar, self.output, pin=pin_for(jar))
            self.assertFalse(self.output.exists())

        with self.subTest(case="case collision"):
            jar = self.root / "case.jar"
            make_jar(jar, {"assets/minecraft/models/Stone.json": b"{}",
                           "assets/minecraft/models/stone.json": b"{}"})
            with self.assertRaises(staging.StageError):
                staging.stage(jar, self.output, pin=pin_for(jar))
            self.assertFalse(self.output.exists())

        with self.subTest(case="symlink"):
            jar = self.root / "link.jar"
            with zipfile.ZipFile(jar, "w", zipfile.ZIP_STORED) as archive:
                archive.writestr("version.json", json.dumps(
                    {"id": staging.PINNED_MINECRAFT,
                     "protocol_version": staging.PINNED_PROTOCOL}))
                info = zipfile.ZipInfo("assets/minecraft/models/link.json")
                info.external_attr = 0o120777 << 16
                archive.writestr(info, "/etc/passwd")
            with self.assertRaises(staging.StageError):
                staging.stage(jar, self.output, pin=pin_for(jar))
            self.assertFalse(self.output.exists())

        with self.subTest(case="referenced traversal"):
            jar = self.root / "ref.jar"
            make_jar(jar, {"assets/minecraft/models/block/x.json": json.dumps(
                {"textures": {"all": "minecraft:../block/stone"}}).encode(),
                "assets/minecraft/textures/block/stone.png": TEXTURE})
            result = staging.stage(jar, self.output, pin=pin_for(jar))
            self.assertEqual(result.references["missing"], [{
                "model": "assets/minecraft/models/block/x.json",
                "texture": "minecraft:../block/stone",
                "expected": None,
            }])
            self.assertFalse(result.references["extra_files"])

    def test_rejects_oversize_selection_before_writing(self):
        per_file = make_jar(
            self.jar,
            {"assets/minecraft/textures/block/big.png":
             b"x" * (staging.MAX_FILE_BYTES + 1)})
        with self.assertRaises(staging.StageError):
            staging.stage(self.jar, self.output, pin=per_file)
        self.assertFalse(self.output.exists())
        self.assertEqual(self.leftovers(), ["client.jar"])

        total_jar = self.root / "total.jar"
        chunk = b"y" * 1_800_000
        total_pin = make_jar(total_jar, {
            f"assets/minecraft/textures/block/part{index}.png": chunk
            for index in range(5)})
        self.assertGreater(5 * len(chunk), staging.MAX_TOTAL_BYTES)
        with self.assertRaises(staging.StageError):
            staging.stage(total_jar, self.output, pin=total_pin)
        self.assertFalse(self.output.exists())

        referenced_jar = self.root / "referenced.jar"
        referenced_pin = make_jar(referenced_jar, {
            "assets/minecraft/models/block/heavy.json": json.dumps(
                {"textures": {f"t{index}": f"item/heavy{index}"
                              for index in range(5)}}).encode(),
            **{f"assets/minecraft/textures/item/heavy{index}.png": chunk
               for index in range(5)},
        })
        with self.assertRaises(staging.StageError):
            staging.stage(referenced_jar, self.output, pin=referenced_pin)
        self.assertFalse(self.output.exists())

        count_jar = self.root / "count.jar"
        count_pin = make_jar(count_jar, {
            f"assets/minecraft/models/m{index:05d}.json": b"{}"
            for index in range(staging.MAX_FILES + 1)})
        with self.assertRaises(staging.StageError):
            staging.stage(count_jar, self.output, pin=count_pin)
        self.assertFalse(self.output.exists())

        # Assets below the cap still fail once the manifest they imply is counted.
        manifest_jar = self.root / "manifest.jar"
        entries = {f"assets/minecraft/models/m{index:05d}.json":
                   b'{"parent":"minecraft:block/cube"}'
                   for index in range(11_000)}
        entries.update({
            f"assets/minecraft/textures/block/p{index}.png": b"z" * 1_600_000
            for index in range(4)})
        asset_bytes = sum(len(data) for data in entries.values())
        self.assertLess(asset_bytes, staging.MAX_TOTAL_BYTES)
        with self.assertRaises(staging.StageError):
            staging.stage(manifest_jar, self.output,
                          pin=make_jar(manifest_jar, entries))
        self.assertFalse(self.output.exists())

    def test_atomic_failure_cleans_temp_and_leaves_no_output(self):
        entries = {"assets/minecraft/models/a.json": b'{"parent":"block/x"}'}
        make_jar(self.jar, entries)
        raw = bytearray(self.jar.read_bytes())
        marker = raw.find(b'{"parent":"block/x"}')
        self.assertGreater(marker, 0)
        raw[marker] ^= 0xFF
        self.jar.write_bytes(raw)
        pin = pin_for(self.jar)

        with self.assertRaises(staging.StageError):
            staging.stage(self.jar, self.output, pin=pin)
        self.assertFalse(self.output.exists())
        self.assertEqual(self.leftovers(), ["client.jar"])
        self.assertFalse(self.output.parent.exists())

    def test_existing_output_is_protected_and_verified(self):
        pin = make_jar(self.jar)
        first = staging.stage(self.jar, self.output, pin=pin)
        self.assertFalse(first.verified_existing)

        second = staging.stage(self.jar, self.output, pin=pin)
        self.assertTrue(second.verified_existing)
        self.assertEqual(second.manifest, first.manifest)
        self.assertEqual(sorted(p.name for p in self.root.iterdir()),
                         ["client.jar", "stage"])

        changed = self.output / "assets/minecraft/textures/block/stone.png"
        changed.write_bytes(TEXTURE + b"tampered")
        with self.assertRaises(staging.StageError):
            staging.stage(self.jar, self.output, pin=pin)
        self.assertEqual(changed.read_bytes(), TEXTURE + b"tampered")
        self.assertEqual(sorted(p.name for p in self.output.parent.iterdir()),
                         ["26.3"])

        changed.write_bytes(TEXTURE)
        staging.stage(self.jar, self.output, pin=pin)

        extra = self.output / "assets/minecraft/models/extra.json"
        extra.write_bytes(b"{}")
        with self.assertRaises(staging.StageError):
            staging.stage(self.jar, self.output, pin=pin)
        self.assertTrue(extra.is_file())

        extra.unlink()
        manifest = json.loads((self.output / "manifest.json").read_text())
        manifest["total_bytes"] += 1
        (self.output / "manifest.json").write_text(
            json.dumps(manifest, sort_keys=True))
        with self.assertRaises(staging.StageError):
            staging.stage(self.jar, self.output, pin=pin)

        bare = self.root / "bare"
        bare.mkdir()
        (bare / "leftover.txt").write_text("mine")
        with self.assertRaises(staging.StageError):
            staging.stage(self.jar, bare, pin=pin)
        self.assertEqual((bare / "leftover.txt").read_text(), "mine")


if __name__ == "__main__":
    unittest.main()
