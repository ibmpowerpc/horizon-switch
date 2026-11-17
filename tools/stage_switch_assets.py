#!/usr/bin/env python3
"""Stage the user's own pinned Java 26.3 render assets for Switch meshing work.

Developer-only, offline and standard library only. The script reads the client
jar the player already installed (never a copy in this repository), checks that
it is the pinned 26.3 build, and extracts a bounded, hashed subset into an
ignored `.local/switch-assets/26.3` folder:

* model/blockstate JSON, and block/colormap PNG textures with `.png.mcmeta`;
* every concrete texture a block model binds, whether written as a string or as
  an object with a `sprite` key, including textures outside `textures/block`
  and `textures/colormap` (plus their `.png.mcmeta`).

Texture references that the jar does not contain are listed in the manifest and
the report; nothing is synthesized, and no atlas baking or model rendering is
attempted. This is a bounded input set for a later Switch meshing experiment,
not a Horizon resource pack.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile
import zipfile

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT = REPO_ROOT / ".local" / "switch-assets" / "26.3"

SCHEMA = 1
PINNED_MINECRAFT = "26.3"
PINNED_PROTOCOL = 777
PINNED_CLIENT_SHA1 = "e877b6a07acd633fb3bb475002175cec036e7b87"

MAX_FILES = 12_000
MAX_FILE_BYTES = 2 * 1024 * 1024
MAX_TOTAL_BYTES = 8 * 1024 * 1024

MANIFEST_NAME = "manifest.json"
CHUNK = 64 * 1024

BLOCK_MODELS = "assets/minecraft/models/block/"
_JSON_DIRS = ("assets/minecraft/models/", "assets/minecraft/blockstates/")
_PNG_DIRS = ("assets/minecraft/textures/block/",
             "assets/minecraft/textures/colormap/")
_SHA1 = re.compile(r"\A[0-9a-f]{40}\Z")


class StageError(Exception):
    """A refusal or a failed staging run; nothing is ever half-committed."""


@dataclass(frozen=True)
class Pin:
    """The client jar this importer accepts, and the protocol it must record."""

    minecraft: str
    protocol: int
    client_sha1: str

    def __post_init__(self):
        if not isinstance(self.minecraft, str) or not self.minecraft:
            raise StageError("pin: minecraft version must be a non-empty string")
        if isinstance(self.protocol, bool) or not isinstance(self.protocol, int):
            raise StageError("pin: protocol must be an integer")
        if self.protocol <= 0:
            raise StageError("pin: protocol must be positive")
        if not _SHA1.match(str(self.client_sha1)):
            raise StageError("pin: client jar SHA1 must be 40 lowercase hex digits")

    @classmethod
    def pinned(cls) -> "Pin":
        """The built-in pin. The CLI never accepts a different one."""
        return cls(PINNED_MINECRAFT, PINNED_PROTOCOL, PINNED_CLIENT_SHA1)

    @classmethod
    def from_research_lock(cls, path: Path) -> "Pin":
        """Confirm, never loosen: a lock file must equal the built-in constants."""
        path = Path(path)
        try:
            raw = json.loads(path.read_text(encoding="utf-8"))
        except OSError as exc:
            raise StageError(f"cannot read research lock {path}: {exc}") from exc
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise StageError(f"research lock {path} is not JSON: {exc}") from exc
        if not isinstance(raw, dict):
            raise StageError(f"research lock {path} must be a JSON object")
        game = raw.get("minecraft")
        if isinstance(game, dict):
            # Horizon research-lock.json nests the jar/version under minecraft;
            # its pinned jar hash also pins the wire protocol.
            lock = cls(str(game.get("version")),
                       game.get("protocol", raw.get("protocol", PINNED_PROTOCOL)),
                       str(game.get("client_sha1")))
        else:
            lock = cls(str(game), raw.get("protocol"),
                       str(raw.get("client_sha1")))
        pinned = cls.pinned()
        if lock != pinned:
            raise StageError(
                f"research lock {path} disagrees with the pinned "
                f"{pinned.minecraft}/protocol {pinned.protocol}/"
                f"SHA1 {pinned.client_sha1}; the lock cannot widen the pin")
        return pinned


@dataclass(frozen=True)
class StageResult:
    output: Path
    source_sha1: str
    count: int
    total_bytes: int
    dry_run: bool
    verified_existing: bool
    references: dict
    manifest: dict | None


def _hash_file(path: Path) -> str:
    digest = hashlib.sha1()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(CHUNK), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _check_source(client_jar: Path, pin: Pin) -> str:
    """Verify the jar's SHA1 and its root version.json id/protocol."""
    if not client_jar.is_file():
        raise StageError(f"client jar not found: {client_jar}")
    actual = _hash_file(client_jar)
    if actual != pin.client_sha1:
        raise StageError(
            f"client jar SHA1 {actual} does not match the pinned "
            f"{pin.client_sha1} ({pin.minecraft})")
    try:
        with zipfile.ZipFile(client_jar) as archive:
            try:
                raw = archive.read("version.json")
            except KeyError as exc:
                raise StageError(
                    "client jar has no root version.json") from exc
    except zipfile.BadZipFile as exc:
        raise StageError(f"client jar is not a readable ZIP: {exc}") from exc
    try:
        version = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise StageError(f"root version.json is not JSON: {exc}") from exc
    if not isinstance(version, dict):
        raise StageError("root version.json is not a JSON object")
    version_id = version.get("id")
    protocol = version.get("protocol_version")
    if version_id != pin.minecraft:
        raise StageError(
            f"client jar version.json id {version_id!r} is not pinned "
            f"{pin.minecraft!r}")
    if isinstance(protocol, bool) or protocol != pin.protocol:
        raise StageError(
            f"client jar version.json protocol_version {protocol!r} is not "
            f"pinned {pin.protocol}")
    return actual


def _check_entry_name(name: str) -> None:
    if not name:
        raise StageError("empty ZIP entry name")
    if "\x00" in name or "\\" in name:
        raise StageError(f"invalid ZIP entry name: {name!r}")
    if name.startswith("/") or (len(name) > 1 and name[1] == ":"):
        raise StageError(f"absolute ZIP entry name: {name!r}")
    if any(part in ("", ".", "..") for part in name.split("/")):
        raise StageError(f"non-canonical ZIP entry name: {name!r}")


def _selected_kind(name: str) -> str | None:
    if any(name.startswith(prefix) for prefix in _JSON_DIRS):
        return "json" if name.endswith(".json") else None
    if any(name.startswith(prefix) for prefix in _PNG_DIRS):
        if name.endswith(".png") or name.endswith(".png.mcmeta"):
            return "png"
    return None


def _sprite_name(value) -> str | None:
    """The concrete texture a `textures` value binds, or None for `#variable`."""
    if isinstance(value, str):
        return None if value.startswith("#") else value
    if isinstance(value, dict):
        sprite = value.get("sprite")
        if isinstance(sprite, str) and sprite and not sprite.startswith("#"):
            return sprite
    return None


def _texture_path(sprite: str) -> str | None:
    """`minecraft:item/apple` -> `assets/minecraft/textures/item/apple.png`."""
    namespace, separator, path = sprite.partition(":")
    if not separator:
        namespace, path = "minecraft", sprite
    if path.endswith(".png"):
        path = path[:-len(".png")]
    if not namespace or not path:
        return None
    if any(part in ("", ".", "..") for part in
           f"{namespace}/{path}".split("/")):
        return None
    return f"assets/{namespace}/textures/{path}.png"


def _validate_entries(archive: zipfile.ZipFile) -> tuple[list[str], dict]:
    """Reject unsafe names, encryption, symlinks and duplicates once."""
    order: list[str] = []
    infos: dict[str, zipfile.ZipInfo] = {}
    seen: set[str] = set()
    for info in archive.infolist():
        name = info.filename
        if info.is_dir():
            continue
        _check_entry_name(name)
        if info.flag_bits & 0x1:
            raise StageError(f"encrypted ZIP entry: {name!r}")
        if (info.external_attr >> 16) & 0o170000 == 0o120000:
            raise StageError(f"symlink ZIP entry: {name!r}")
        if name in seen or name.casefold() in seen:
            raise StageError(f"duplicate ZIP entry: {name!r}")
        seen.add(name)
        seen.add(name.casefold())
        infos[name] = info
        order.append(name)
    return order, infos


def _read_entry(archive: zipfile.ZipFile, info: zipfile.ZipInfo) -> bytes:
    """One bounded read; the ZIP reader verifies the stored CRC at EOF."""
    try:
        with archive.open(info) as stream:
            data = stream.read(MAX_FILE_BYTES + 1)
    except zipfile.BadZipFile as exc:
        raise StageError(f"corrupt ZIP entry {info.filename!r}: {exc}") from exc
    except (RuntimeError, NotImplementedError, OSError) as exc:
        raise StageError(f"unreadable ZIP entry {info.filename!r}: {exc}") from exc
    if len(data) > MAX_FILE_BYTES:
        raise StageError(
            f"ZIP entry {info.filename!r} exceeds the "
            f"{MAX_FILE_BYTES}-byte per-file limit")
    if len(data) != info.file_size:
        raise StageError(
            f"ZIP entry {info.filename!r} size {len(data)} disagrees with its "
            f"central directory ({info.file_size})")
    return data


def _resolve_textures(archive: zipfile.ZipFile, infos: dict,
                      order: list[str], selected: list[str]) -> tuple[list[str], dict]:
    """Follow block-model texture bindings, collecting extra PNGs and misses."""
    chosen = set(selected)
    extras: list[str] = []
    missing: list[dict] = []
    models = sorted(name for name in order
                    if name.startswith(BLOCK_MODELS) and name.endswith(".json"))
    bindings = 0
    for model in models:
        try:
            document = json.loads(
                _read_entry(archive, infos[model]).decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise StageError(f"block model {model!r} is not JSON: {exc}") from exc
        textures = document.get("textures") if isinstance(document, dict) else None
        if not isinstance(textures, dict):
            continue
        for value in textures.values():
            sprite = _sprite_name(value)
            if sprite is None:
                if isinstance(value, dict):
                    missing.append({
                        "model": model,
                        "texture": json.dumps(value, sort_keys=True),
                        "expected": None,
                    })
                continue
            bindings += 1
            expected = _texture_path(sprite)
            if expected is None:
                missing.append({"model": model, "texture": sprite,
                                "expected": None})
                continue
            if expected not in infos:
                missing.append({"model": model, "texture": sprite,
                                "expected": expected})
                continue
            for name in (expected, expected + ".mcmeta"):
                if name in infos and name not in chosen:
                    chosen.add(name)
                    extras.append(name)
    extras.sort()
    missing.sort(key=lambda item: (item["model"], item["texture"]))
    references = {
        "models_scanned": len(models),
        "texture_bindings": bindings,
        "extra_files": extras,
        "missing": missing,
    }
    return extras, references


def _check_budget(selected: list[str], infos: dict) -> int:
    total = 0
    for name in selected:
        size = infos[name].file_size
        if size > MAX_FILE_BYTES:
            raise StageError(
                f"ZIP entry {name!r} declares {size} bytes, over the "
                f"{MAX_FILE_BYTES}-byte per-file limit")
        total += size
        if total > MAX_TOTAL_BYTES:
            raise StageError(
                f"selection exceeds the {MAX_TOTAL_BYTES}-byte output limit")
    if len(selected) > MAX_FILES:
        raise StageError(f"selection exceeds the {MAX_FILES}-file limit")
    return total


def _plan(archive: zipfile.ZipFile, pin: Pin) -> tuple[list[zipfile.ZipInfo], dict]:
    """Validate every entry, select the pinned subset, resolve its textures."""
    order, infos = _validate_entries(archive)
    base = [name for name in order if _selected_kind(name) is not None]
    extras, references = _resolve_textures(archive, infos, order, base)
    selected = sorted(base + extras)
    if not selected:
        raise StageError("client jar contains no pinned render assets")
    total = _check_budget(selected, infos)
    # The manifest lives beside the assets, so the output cap covers both. Its
    # length is fixed here: paths, sizes and 64-character hashes are known.
    skeleton = _manifest(pin, "0" * 40, [
        {"path": name, "size": infos[name].file_size, "sha256": "0" * 64}
        for name in selected], total, references)
    if total + len(_manifest_bytes(skeleton)) > MAX_TOTAL_BYTES:
        raise StageError(
            f"selection plus manifest exceeds the {MAX_TOTAL_BYTES}-byte "
            f"output limit")
    return [infos[name] for name in selected], references


def _stream(archive: zipfile.ZipFile, selected: list[zipfile.ZipInfo],
            destination: Path | None) -> tuple[list[dict], int]:
    """Hash every selected entry, writing it only when a destination is given."""
    files: list[dict] = []
    total = 0
    for info in selected:
        data = _read_entry(archive, info)
        total += len(data)
        if total > MAX_TOTAL_BYTES:
            raise StageError(
                f"selection exceeds the {MAX_TOTAL_BYTES}-byte output limit")
        if destination is not None:
            target = destination.joinpath(*info.filename.split("/"))
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        files.append({
            "path": info.filename,
            "size": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        })
    return files, total


def _manifest(pin: Pin, source_sha1: str, files: list[dict], total: int,
              references: dict) -> dict:
    return {
        "schema": SCHEMA,
        "minecraft": pin.minecraft,
        "protocol": pin.protocol,
        "source_sha1": source_sha1,
        "count": len(files),
        "total_bytes": total,
        "texture_references": references,
        "files": files,
    }


def _manifest_bytes(manifest: dict) -> bytes:
    return (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")


def _verify_existing(output: Path, manifest: dict) -> None:
    """Accept an existing output only if its manifest and bytes are identical."""
    if output.is_symlink() or not output.is_dir():
        raise StageError(f"refusing existing output {output}: not a plain directory")
    manifest_path = output / MANIFEST_NAME
    try:
        existing = json.loads(manifest_path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise StageError(
            f"refusing existing output {output}: no readable "
            f"{MANIFEST_NAME} ({exc})") from exc
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise StageError(
            f"refusing existing output {output}: {MANIFEST_NAME} is not JSON "
            f"({exc})") from exc
    if existing != manifest:
        raise StageError(
            f"refusing existing output {output}: it does not match this client "
            f"jar; remove it first")
    expected = {entry["path"] for entry in manifest["files"]}
    for entry in manifest["files"]:
        path = output.joinpath(*entry["path"].split("/"))
        try:
            data = path.read_bytes()
        except OSError as exc:
            raise StageError(
                f"refusing existing output {output}: {entry['path']} is "
                f"unreadable ({exc})") from exc
        if len(data) != entry["size"] or \
                hashlib.sha256(data).hexdigest() != entry["sha256"]:
            raise StageError(
                f"refusing existing output {output}: {entry['path']} differs "
                f"from the manifest")
    for path in output.rglob("*"):
        if path.is_dir() and not path.is_symlink():
            continue
        relative = path.relative_to(output).as_posix()
        if relative != MANIFEST_NAME and relative not in expected:
            raise StageError(
                f"refusing existing output {output}: unexpected {relative!r}")


def _missing_dirs(path: Path) -> list[Path]:
    """Create `path`, returning the directories that did not exist before."""
    missing: list[Path] = []
    probe = path
    while not probe.exists():
        if probe.is_symlink():
            raise StageError(f"refusing to create directories through {probe}")
        missing.append(probe)
        if probe.parent == probe:
            break
        probe = probe.parent
    if not probe.is_dir():
        raise StageError(f"cannot create {path}: {probe} is not a directory")
    try:
        for directory in reversed(missing):
            directory.mkdir()
    except OSError as exc:
        for directory in missing:
            try:
                directory.rmdir()
            except OSError:
                pass
        raise StageError(f"cannot create {path}: {exc}") from exc
    return missing


def _prune_dirs(created: list[Path]) -> None:
    for directory in created:
        try:
            directory.rmdir()
        except OSError:
            pass


def stage(client_jar: Path, output: Path, *, pin: Pin | None = None,
          dry_run: bool = False) -> StageResult:
    """Plan, validate and (unless `dry_run`) atomically stage the assets."""
    pin = Pin.pinned() if pin is None else pin
    client_jar = Path(client_jar)
    output = Path(output)
    source_sha1 = _check_source(client_jar, pin)
    with zipfile.ZipFile(client_jar) as archive:
        selected, references = _plan(archive, pin)
        declared = sum(info.file_size for info in selected)
        if dry_run:
            return StageResult(output, source_sha1, len(selected), declared,
                               True, False, references, None)

        if output.exists() or output.is_symlink():
            files, total = _stream(archive, selected, None)
            manifest = _manifest(pin, source_sha1, files, total, references)
            _verify_existing(output, manifest)
            return StageResult(output, source_sha1, len(files), total,
                               False, True, references, manifest)

        created = _missing_dirs(output.parent)
        temporary: Path | None = None
        try:
            temporary = Path(tempfile.mkdtemp(
                prefix=f".{output.name}.staging-", dir=str(output.parent)))
            files, total = _stream(archive, selected, temporary)
            manifest = _manifest(pin, source_sha1, files, total, references)
            (temporary / MANIFEST_NAME).write_bytes(_manifest_bytes(manifest))
            if os.path.lexists(output):
                raise StageError(f"refusing to overwrite {output}")
            os.replace(temporary, output)
            temporary = None
        except BaseException:
            if temporary is not None:
                shutil.rmtree(temporary, ignore_errors=True)
            _prune_dirs(created)
            raise
        return StageResult(output, source_sha1, len(files), total,
                           False, False, references, manifest)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client-jar", type=Path, required=True,
                        help="the player's own minecraft-26.3-client.jar")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT,
                        help=f"staging folder (default {DEFAULT_OUTPUT})")
    parser.add_argument("--dry-run", action="store_true",
                        help="report the selection without extracting anything")
    parser.add_argument("--research-lock", type=Path,
                        help="JSON pin that must repeat the built-in constants")
    return parser


def _report_references(references: dict) -> None:
    print(f"block models scanned: {references['models_scanned']}, "
          f"concrete texture bindings: {references['texture_bindings']}, "
          f"extra referenced files: {len(references['extra_files'])}, "
          f"missing texture references: {len(references['missing'])}")
    for item in references["missing"]:
        target = item["expected"] or "(unresolvable)"
        print(f"missing: {item['model']} -> {item['texture']} ({target})")
    for path in references["extra_files"]:
        print(f"referenced: {path}")


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        pin = (Pin.from_research_lock(args.research_lock)
               if args.research_lock is not None else Pin.pinned())
        result = stage(args.client_jar, args.output, pin=pin,
                       dry_run=args.dry_run)
    except StageError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    limits = (f"{MAX_FILES} files, {MAX_FILE_BYTES} bytes each, "
              f"{MAX_TOTAL_BYTES} bytes total")
    if result.dry_run:
        print(f"dry-run: {args.client_jar}")
        print(f"source SHA1 {result.source_sha1} "
              f"(pinned {pin.minecraft}, protocol {pin.protocol})")
        print(f"selected {result.count} entries, {result.total_bytes} bytes "
              f"declared in the jar (limits: {limits})")
        print(f"no files written; would stage into {result.output}")
    elif result.verified_existing:
        print(f"verified existing {result.output}: {result.count} files, "
              f"{result.total_bytes} bytes (unchanged)")
    else:
        print(f"staged {result.count} files, {result.total_bytes} bytes into "
              f"{result.output}")
    _report_references(result.references)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
