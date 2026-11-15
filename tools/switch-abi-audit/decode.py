#!/usr/bin/env python3
"""Decode the compile-only Switch C ABI probe and compare it with Rust libc.

The C side is measured: devkitA64 GCC emits uint64 size/alignment/offsetof
records into the .switch_abi ELF section, objcopy dumps them, and this script
decodes the little-endian table.

The Rust side is source-derived: layouts are computed from the installed
libc 0.2.189 sources for target_os="horizon".  Nothing is compiled or executed
on either side.
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import re
import struct
import sys
from datetime import datetime, timezone

MAGIC = 0x314942412D484357  # "WCH-ABI1" as little-endian bytes
SCHEMA = 1
RECORD_WORDS = 3

PRIMITIVES = {
    "c_char": (1, 1),
    "c_schar": (1, 1),
    "c_uchar": (1, 1),
    "c_short": (2, 2),
    "c_ushort": (2, 2),
    "c_int": (4, 4),
    "c_uint": (4, 4),
    "c_long": (8, 8),
    "c_ulong": (8, 8),
    "c_longlong": (8, 8),
    "c_ulonglong": (8, 8),
    "i8": (1, 1),
    "u8": (1, 1),
    "i16": (2, 2),
    "u16": (2, 2),
    "i32": (4, 4),
    "u32": (4, 4),
    "i64": (8, 8),
    "u64": (8, 8),
    "isize": (8, 8),
    "usize": (8, 8),
    "size_t": (8, 8),
    "ssize_t": (8, 8),
    "f32": (4, 4),
    "f64": (8, 8),
    "u128": (16, 16),
    "i128": (16, 16),
}


class AuditError(RuntimeError):
    pass


def align_up(value: int, alignment: int) -> int:
    return (value + alignment - 1) // alignment * alignment


def line_at(text: str, pos: int) -> int:
    return text.count("\n", 0, pos) + 1


def read_text(path: str) -> str:
    with open(path, "r", encoding="utf-8") as handle:
        return handle.read()


def read_kv(path: str) -> dict[str, str]:
    values: dict[str, str] = {}
    if not path or not os.path.exists(path):
        return values
    with open(path, "r", encoding="utf-8") as handle:
        for raw in handle:
            line = raw.strip()
            if not line or "=" not in line:
                continue
            key, value = line.split("=", 1)
            values[key.strip()] = value.strip()
    return values


def parse_def(path: str) -> list[dict[str, object]]:
    items: list[dict[str, object]] = []
    with open(path, "r", encoding="utf-8") as handle:
        for lineno, raw in enumerate(handle, 1):
            line = raw.split("//", 1)[0].strip()
            if not line:
                continue
            match = re.fullmatch(r"(TYPE|FIELD)\(([^)]*)\)", line)
            if not match:
                raise AuditError(f"{path}:{lineno}: cannot parse {line!r}")
            args = [part.strip() for part in match.group(2).split(",")]
            if match.group(1) == "TYPE":
                if len(args) != 1 or not args[0]:
                    raise AuditError(f"{path}:{lineno}: bad TYPE item")
                items.append({"kind": "type", "name": args[0], "def_line": lineno})
            else:
                if len(args) != 2 or not all(args):
                    raise AuditError(f"{path}:{lineno}: bad FIELD item")
                items.append(
                    {
                        "kind": "field",
                        "struct": args[0],
                        "name": args[1],
                        "def_line": lineno,
                    }
                )
    if not items:
        raise AuditError(f"{path}: empty item list")
    return items


def decode_table(path: str, items: list[dict[str, object]]) -> dict[str, object]:
    with open(path, "rb") as handle:
        blob = handle.read()
    if len(blob) % 8 != 0:
        raise AuditError(f"{path}: byte count {len(blob)} is not a multiple of 8")
    words = struct.unpack("<%dQ" % (len(blob) // 8), blob)
    if len(words) < 3:
        raise AuditError(f"{path}: truncated table")
    magic, schema, count = words[:3]
    if magic != MAGIC:
        raise AuditError(f"{path}: bad magic 0x{magic:016x}")
    if schema != SCHEMA:
        raise AuditError(f"{path}: unsupported schema {schema}")
    payload = words[3:]
    if count != len(items):
        raise AuditError(f"{path}: header count {count} != {len(items)} items")
    if len(payload) != count * RECORD_WORDS:
        raise AuditError(
            f"{path}: payload has {len(payload)} words, expected {count * RECORD_WORDS}"
        )

    types: dict[str, object] = {}
    structs: dict[str, object] = {}
    records: list[dict[str, object]] = []

    for index, item in enumerate(items):
        base = index * RECORD_WORDS
        kind, value_a, value_b = payload[base : base + RECORD_WORDS]
        expected_kind = 1 if item["kind"] == "type" else 2
        if kind != expected_kind:
            raise AuditError(
                f"record {index}: kind {kind}, expected {expected_kind} for {item}"
            )

        if item["kind"] == "type":
            name = str(item["name"])
            entry = {
                "size": int(value_a),
                "align": int(value_b),
                "def_line": item["def_line"],
            }
            records.append({"name": name, **entry})
            if name.startswith("struct "):
                struct_name = name[len("struct ") :]
                if value_b == 0 or value_b & (value_b - 1):
                    raise AuditError(f"{name}: invalid alignment {value_b}")
                structs[struct_name] = {
                    "size": int(value_a),
                    "align": int(value_b),
                    "fields": {},
                    "def_line": item["def_line"],
                }
            else:
                types[name] = entry
        else:
            struct_name = str(item["struct"])
            struct_key = struct_name[len("struct ") :] if struct_name.startswith("struct ") else struct_name
            field_name = str(item["name"])
            if struct_key not in structs:
                raise AuditError(
                    f"field {struct_name}.{field_name} appears before its struct type"
                )
            structs[struct_key]["fields"][field_name] = {
                "offset": int(value_a),
                "def_line": item["def_line"],
            }
            records.append(
                {
                    "struct": struct_key,
                    "name": field_name,
                    "offset": int(value_a),
                }
            )

    return {
        "magic": f"0x{magic:016x}",
        "schema": int(schema),
        "record_count": int(count),
        "bytes": len(blob),
        "types": types,
        "structs": structs,
        "records": records,
    }


def balanced_body(text: str, open_index: int) -> tuple[str, int]:
    depth = 0
    for index in range(open_index, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[open_index + 1 : index], index
    raise AuditError("unbalanced braces in Rust struct")


def keep_field(attributes: list[str]) -> bool:
    joined = " ".join(attributes)
    if "not(" in joined:
        return True
    for needle in (
        'target_os = "espidf"',
        'target_os = "vita"',
        'target_arch = "x86_64"',
        'target_arch = "powerpc"',
        'target_pointer_width = "32"',
        "musl32_time64",
    ):
        if needle in joined:
            return False
    return True


def parse_structs(text: str, source: str) -> dict[str, dict[str, object]]:
    found: dict[str, dict[str, object]] = {}
    pattern = re.compile(r"pub struct\s+(\w+)\s*\{")
    for match in pattern.finditer(text):
        name = match.group(1)
        body, _ = balanced_body(text, match.end() - 1)
        fields: list[tuple[str, str]] = []
        attributes: list[str] = []
        for raw in body.splitlines():
            line = raw.strip()
            if not line or line.startswith("//"):
                continue
            if line.startswith("#["):
                attributes.append(line)
                continue
            field = re.match(r"(?:pub\s+)?(\w+)\s*:\s*(.+?),\s*$", line)
            if not field:
                attributes = []
                continue
            if keep_field(attributes):
                fields.append((field.group(1), field.group(2).strip()))
            attributes = []
        found[name] = {
            "size": None,
            "align": None,
            "fields": fields,
            "offsets": {},
            "source": source,
            "line": line_at(text, match.start()),
        }
    return found


def collect_aliases(text: str, source: str) -> dict[str, list[dict[str, object]]]:
    aliases: dict[str, list[dict[str, object]]] = {}
    pattern = re.compile(r"\bpub type\s+(\w+)\s*=\s*([^;]+);")
    for match in pattern.finditer(text):
        name = match.group(1)
        aliases.setdefault(name, []).append(
            {
                "value": match.group(2).strip(),
                "source": source,
                "line": line_at(text, match.start()),
                "pos": match.start(),
            }
        )
    return aliases


class RustModel:
    """Source-derived layout model for libc 0.2.189 on aarch64 Horizon."""

    def __init__(self, libc_src: str):
        self.root = libc_src
        self.files = {
            "unix/mod.rs": read_text(os.path.join(libc_src, "src/unix/mod.rs")),
            "unix/newlib/mod.rs": read_text(
                os.path.join(libc_src, "src/unix/newlib/mod.rs")
            ),
            "unix/newlib/horizon/mod.rs": read_text(
                os.path.join(libc_src, "src/unix/newlib/horizon/mod.rs")
            ),
            "unix/newlib/generic.rs": read_text(
                os.path.join(libc_src, "src/unix/newlib/generic.rs")
            ),
        }
        self.aliases: dict[str, str] = {}
        self.alias_refs: dict[str, dict[str, object]] = {}
        self.structs: dict[str, dict[str, object]] = {}

        newlib_aliases = collect_aliases(
            self.files["unix/newlib/mod.rs"], "unix/newlib/mod.rs"
        )
        unix_aliases = collect_aliases(self.files["unix/mod.rs"], "unix/mod.rs")

        self._choose(
            newlib_aliases,
            self.files["unix/newlib/mod.rs"],
            {
                "sa_family_t": "horizon",
                "time_t": "horizon",
                "dev_t": "last",
                "ino_t": "last",
                "off_t": "last",
            },
        )
        self._choose(
            unix_aliases,
            self.files["unix/mod.rs"],
            {"uid_t": "first", "gid_t": "first"},
        )

        self._add_structs(
            "unix/mod.rs", ["timespec"], self.files["unix/mod.rs"]
        )
        self._add_structs(
            "unix/newlib/generic.rs",
            ["dirent"],
            self.files["unix/newlib/generic.rs"],
        )
        self._add_structs(
            "unix/newlib/mod.rs",
            ["addrinfo", "in_addr"],
            self.files["unix/newlib/mod.rs"],
        )
        self._add_structs(
            "unix/newlib/horizon/mod.rs",
            ["sockaddr", "sockaddr_in", "sockaddr_storage", "stat"],
            self.files["unix/newlib/horizon/mod.rs"],
        )

        self.pthread = self._pthread_layouts()

    def _choose(
        self,
        occurrences: dict[str, list[dict[str, object]]],
        text: str,
        strategies: dict[str, str],
    ) -> None:
        for name, choices in occurrences.items():
            strategy = strategies.get(name, "only")
            if strategy == "horizon":
                filtered = [
                    item
                    for item in choices
                    if 'target_os = "horizon"'
                    in text[max(0, int(item["pos"]) - 500) : int(item["pos"])]
                ]
                chosen = filtered[0] if filtered else choices[-1]
            elif strategy == "last":
                chosen = choices[-1]
            elif strategy == "first":
                chosen = choices[0]
            else:
                if len(choices) != 1:
                    continue
                chosen = choices[0]
            self.aliases[name] = str(chosen["value"])
            self.alias_refs[name] = {
                "source": chosen["source"],
                "line": chosen["line"],
                "value": str(chosen["value"]),
            }

    def _add_structs(
        self, source: str, names: list[str], text: str
    ) -> None:
        parsed = parse_structs(text, source)
        for name in names:
            if name not in parsed:
                raise AuditError(f"{source}: struct {name} not found")
            self.structs[name] = parsed[name]

    def _pthread_layouts(self) -> dict[str, dict[str, object]]:
        text = self.files["unix/newlib/mod.rs"]
        sizes = {
            "pthread_attr_t": ("__SIZEOF_PTHREAD_ATTR_T", 1),
            "pthread_mutex_t": ("__SIZEOF_PTHREAD_MUTEX_T", 8),
            "pthread_cond_t": ("__SIZEOF_PTHREAD_COND_T", 8),
            "pthread_mutexattr_t": ("__SIZEOF_PTHREAD_MUTEXATTR_T", 8),
            "pthread_condattr_t": ("__SIZEOF_PTHREAD_CONDATTR_T", 4),
            "pthread_key_t": (None, 4),
        }
        result: dict[str, dict[str, object]] = {}
        for name, (constant, alignment) in sizes.items():
            if constant is None:
                size, align = self.resolve("pthread_key_t")
                result[name] = {
                    "size": size,
                    "align": align,
                    "source": "unix/newlib/mod.rs",
                    "line": self.alias_refs.get("pthread_key_t", {}).get("line"),
                    "note": "type alias",
                }
                continue
            matches = list(
                re.finditer(
                    rf"pub const {constant}: usize = (\d+);",
                    text,
                )
            )
            if not matches:
                raise AuditError(f"missing {constant}")
            chosen = matches[-1]
            result[name] = {
                "size": align_up(int(chosen.group(1)), alignment),
                "align": alignment,
                "source": "unix/newlib/mod.rs",
                "line": line_at(text, chosen.start()),
                "note": "size from __SIZEOF_* else branch; align from repr attribute",
            }
        return result

    def resolve(self, type_text: str, seen: tuple[str, ...] = ()) -> tuple[int, int]:
        ty = re.sub(r"\s+", " ", type_text.strip())
        if ty.startswith("Padding<") and ty.endswith(">"):
            return self.resolve(ty[len("Padding<") : -1], seen)
        array = re.fullmatch(r"\[(.+?);\s*(\d+)(?:usize)?\s*\]", ty)
        if array:
            size, align = self.resolve(array.group(1), seen)
            return size * int(array.group(2)), align
        if "*" in ty:
            return 8, 8
        ty = ty.removeprefix("crate::")
        if ty in PRIMITIVES:
            return PRIMITIVES[ty]
        if ty in self.aliases:
            if ty in seen:
                raise AuditError(f"alias cycle at {ty}")
            return self.resolve(self.aliases[ty], seen + (ty,))
        if ty in self.structs:
            return self.struct_layout(ty)
        raise AuditError(f"cannot resolve Rust type {type_text!r}")

    def struct_layout(self, name: str) -> tuple[int, int]:
        struct = self.structs[name]
        if struct["size"] is not None:
            return int(struct["size"]), int(struct["align"])
        offset = 0
        alignment = 1
        offsets: dict[str, int] = {}
        for field_name, field_type in struct["fields"]:
            size, field_align = self.resolve(field_type)
            offset = align_up(offset, field_align)
            offsets[field_name] = offset
            offset += size
            alignment = max(alignment, field_align)
        struct["offsets"] = offsets
        struct["align"] = alignment
        struct["size"] = align_up(offset, alignment)
        return int(struct["size"]), int(struct["align"])


def load_rust_model(libc_src: str | None) -> tuple[RustModel | None, dict[str, object]]:
    info: dict[str, object] = {"available": False, "label": "source-derived, not measured"}
    candidates: list[str] = []
    if libc_src:
        candidates.append(libc_src)
    else:
        pattern = os.path.expanduser(
            "~/.cargo/registry/src/*/libc-0.2.189"
        )
        candidates.extend(sorted(glob.glob(pattern)))
    candidates = [path for path in candidates if os.path.isdir(path)]
    if not candidates:
        info["error"] = (
            "libc-0.2.189 sources not found; pass --libc-src or set LIBC_SRC"
        )
        return None, info
    try:
        model = RustModel(candidates[0])
    except (OSError, AuditError) as error:
        info["error"] = str(error)
        info["path"] = candidates[0]
        return None, info
    info.update(
        {
            "available": True,
            "path": candidates[0],
            "version": "0.2.189",
            "target": 'aarch64-nintendo-switch, target_os="horizon"',
            "method": (
                "Rust struct field lists and type aliases parsed from the "
                "installed libc sources; size/alignment/offsets computed with "
                "C-like repr(C) layout for aarch64. No Rust code was compiled "
                "or executed."
            ),
            "files": [
                "src/unix/mod.rs",
                "src/unix/newlib/mod.rs",
                "src/unix/newlib/generic.rs",
                "src/unix/newlib/horizon/mod.rs",
            ],
            "alias_line_refs": model.alias_refs,
        }
    )
    return model, info


def rust_struct_facts(model: RustModel, name: str) -> dict[str, object] | None:
    if name not in model.structs:
        return None
    struct = model.structs[name]
    size, align = model.struct_layout(name)
    return {
        "size": size,
        "align": align,
        "fields": dict(struct["offsets"]),
        "source": struct["source"],
        "line": struct["line"],
    }


def compare(
    c_table: dict[str, object], model: RustModel
) -> dict[str, object]:
    c_types = c_table["types"]
    c_structs = c_table["structs"]

    type_names = [
        "dev_t",
        "ino_t",
        "off_t",
        "blksize_t",
        "blkcnt_t",
        "sa_family_t",
        "socklen_t",
        "time_t",
        "mode_t",
        "nlink_t",
        "uid_t",
        "gid_t",
        "in_port_t",
        "in_addr_t",
    ]
    type_rows: list[dict[str, object]] = []
    for name in type_names:
        if name not in c_types:
            continue
        rust_size, rust_align = model.resolve(name)
        c = c_types[name]
        type_rows.append(
            {
                "name": name,
                "c": {"size": c["size"], "align": c["align"]},
                "rust": {"size": rust_size, "align": rust_align},
                "match": c["size"] == rust_size and c["align"] == rust_align,
                "rust_source": model.alias_refs.get(name),
            }
        )

    struct_names = [
        "stat",
        "dirent",
        "sockaddr",
        "sockaddr_in",
        "sockaddr_storage",
        "addrinfo",
        "timespec",
    ]
    struct_rows: list[dict[str, object]] = []
    field_rows: list[dict[str, object]] = []
    only_rows: list[dict[str, object]] = []
    for name in struct_names:
        if name not in c_structs:
            continue
        rust = rust_struct_facts(model, name)
        if rust is None:
            continue
        c = c_structs[name]
        struct_rows.append(
            {
                "name": name,
                "c": {"size": c["size"], "align": c["align"]},
                "rust": {"size": rust["size"], "align": rust["align"]},
                "match": c["size"] == rust["size"] and c["align"] == rust["align"],
                "rust_source": {"source": rust["source"], "line": rust["line"]},
            }
        )
        c_fields = c["fields"]
        rust_fields = rust["fields"]
        for field_name in c_fields:
            if field_name in rust_fields:
                field_rows.append(
                    {
                        "name": f"{name}.{field_name}",
                        "c_offset": c_fields[field_name]["offset"],
                        "rust_offset": rust_fields[field_name],
                        "match": c_fields[field_name]["offset"]
                        == rust_fields[field_name],
                    }
                )
            else:
                only_rows.append(
                    {
                        "name": f"{name}.{field_name}",
                        "present_in": "C only",
                        "note": "field absent from the Rust definition",
                    }
                )
        for field_name in rust_fields:
            if field_name not in c_fields:
                only_rows.append(
                    {
                        "name": f"{name}.{field_name}",
                        "present_in": "Rust only",
                        "note": "field absent from the C headers",
                    }
                )

    pthread_names = [
        "pthread_mutex_t",
        "pthread_cond_t",
        "pthread_attr_t",
        "pthread_key_t",
        "pthread_mutexattr_t",
        "pthread_condattr_t",
    ]
    pthread_rows: list[dict[str, object]] = []
    for name in pthread_names:
        if name not in c_types or name not in model.pthread:
            continue
        c = c_types[name]
        rust = model.pthread[name]
        pthread_rows.append(
            {
                "name": name,
                "c": {"size": c["size"], "align": c["align"]},
                "rust": {"size": rust["size"], "align": rust["align"]},
                "match": c["size"] == rust["size"]
                and c["align"] == rust["align"],
                "rust_source": {
                    "source": rust["source"],
                    "line": rust["line"],
                    "note": rust["note"],
                },
            }
        )

    all_rows = type_rows + struct_rows + pthread_rows
    field_mismatches = [row for row in field_rows if not row["match"]]
    return {
        "types": type_rows,
        "structs": struct_rows,
        "fields": field_rows,
        "field_mismatches": field_mismatches,
        "unpaired_fields": only_rows,
        "pthread": pthread_rows,
        "summary": {
            "layout_compared": len(all_rows),
            "layout_matched": sum(1 for row in all_rows if row["match"]),
            "layout_mismatched": sum(1 for row in all_rows if not row["match"]),
            "fields_compared": len(field_rows),
            "fields_mismatched": len(field_mismatches),
        },
    }


def fmt_size_align(value: dict[str, object]) -> str:
    return f"{value['size']}/{value['align']}"


def build_report(
    c_table: dict[str, object],
    comparison: dict[str, object] | None,
    rust_info: dict[str, object],
    env: dict[str, str],
    image: dict[str, str],
) -> str:
    lines: list[str] = []
    lines.append("Switch C ABI audit (compile-only, no target execution)")
    lines.append(
        f"Image: {image.get('image', 'unknown')} id={image.get('image_id', '?')}"
    )
    if image.get("image_repo_digest"):
        lines.append(f"Digest: {image['image_repo_digest']}")
    lines.append(
        "Compiler: "
        f"{env.get('compiler', 'unknown')} [{env.get('gcc_target', '?')}]"
    )
    if env.get("cflags"):
        lines.append(f"Flags: {env['cflags']}")
    lines.append(
        f"Measured: {c_table['record_count']} records, {c_table['bytes']} bytes "
        f"({c_table['magic']}) from .switch_abi"
    )
    lines.append("")

    c_types = c_table["types"]
    lines.append("Measured C types (size/align):")
    type_line = "  ".join(
        f"{name}={fmt_size_align(c_types[name])}"
        for name in [
            "dev_t",
            "ino_t",
            "off_t",
            "blksize_t",
            "blkcnt_t",
            "sa_family_t",
            "socklen_t",
            "time_t",
            "mode_t",
            "nlink_t",
            "uid_t",
            "gid_t",
        ]
        if name in c_types
    )
    lines.append("  " + type_line)
    c_structs = c_table["structs"]
    lines.append("Measured C structs (size/align):")
    lines.append(
        "  "
        + "  ".join(
            f"{name}={c_structs[name]['size']}/{c_structs[name]['align']}"
            for name in [
                "stat",
                "dirent",
                "sockaddr",
                "sockaddr_in",
                "sockaddr_storage",
                "addrinfo",
                "timespec",
            ]
            if name in c_structs
        )
    )
    lines.append(
        "  pthread: "
        + "  ".join(
            f"{name}={fmt_size_align(c_types[name])}"
            for name in [
                "pthread_mutex_t",
                "pthread_cond_t",
                "pthread_attr_t",
                "pthread_key_t",
                "pthread_mutexattr_t",
                "pthread_condattr_t",
            ]
            if name in c_types
        )
    )
    lines.append("")
    lines.append("Measured key offsets:")
    grouped_offsets = [
        (
            "stat",
            [
                "st_dev",
                "st_ino",
                "st_mode",
                "st_nlink",
                "st_uid",
                "st_gid",
                "st_rdev",
                "st_size",
                "st_atim",
                "st_mtim",
                "st_ctim",
                "st_blksize",
                "st_blocks",
                "st_spare4",
            ],
        ),
        ("dirent", ["d_ino", "d_type", "d_name"]),
        ("in_addr", ["s_addr"]),
        ("sockaddr", ["sa_len", "sa_family", "sa_data"]),
        ("sockaddr_in", ["sin_len", "sin_family", "sin_port", "sin_addr", "sin_zero"]),
        (
            "sockaddr_storage",
            ["ss_len", "ss_family", "__ss_pad1", "__ss_align", "__ss_pad2"],
        ),
        (
            "addrinfo",
            [
                "ai_flags",
                "ai_family",
                "ai_socktype",
                "ai_protocol",
                "ai_addrlen",
                "ai_canonname",
                "ai_addr",
                "ai_next",
            ],
        ),
        ("timespec", ["tv_sec", "tv_nsec"]),
    ]
    for struct_name, field_names in grouped_offsets:
        struct = c_structs.get(struct_name)
        if not struct:
            continue
        parts = [
            f"{field_name}={struct['fields'][field_name]['offset']}"
            for field_name in field_names
            if field_name in struct["fields"]
        ]
        lines.append(f"  {struct_name}: " + "  ".join(parts))
    lines.append("")

    if not comparison:
        lines.append("Rust comparison unavailable:")
        lines.append(f"  {rust_info.get('error', 'unknown error')}")
        return "\n".join(lines) + "\n"

    lines.append("Rust libc 0.2.189 comparison (SOURCE-DERIVED, NOT MEASURED)")
    lines.append(f"  Source: {rust_info.get('path', '?')}")
    lines.append(
        "  Model: fields/aliases parsed; repr(C) layout for aarch64; no Rust build"
    )
    lines.append("")

    lines.append("Type size/alignment mismatches:")
    type_mismatches = [row for row in comparison["types"] if not row["match"]]
    if type_mismatches:
        for row in type_mismatches:
            lines.append(
                f"  {row['name']}: C {fmt_size_align(row['c'])} vs "
                f"Rust {fmt_size_align(row['rust'])}"
            )
    else:
        lines.append("  none")
    lines.append(
        f"  types matched: {sum(1 for row in comparison['types'] if row['match'])}"
        f"/{len(comparison['types'])}"
    )

    lines.append("Struct size/alignment:")
    for row in comparison["structs"]:
        mark = "ok" if row["match"] else "MISMATCH"
        lines.append(
            f"  {row['name']}: C {fmt_size_align(row['c'])} vs "
            f"Rust {fmt_size_align(row['rust'])} [{mark}]"
        )
    lines.append(
        "  A matching size/alignment does not imply field compatibility; see offsets."
    )

    lines.append("Field offset mismatches:")
    if comparison["field_mismatches"]:
        for row in comparison["field_mismatches"]:
            lines.append(
                f"  {row['name']}: C {row['c_offset']} vs Rust {row['rust_offset']}"
            )
    else:
        lines.append("  none")
    lines.append(
        f"  fields matched: {comparison['summary']['fields_compared'] - comparison['summary']['fields_mismatched']}"
        f"/{comparison['summary']['fields_compared']}"
    )
    if comparison["unpaired_fields"]:
        lines.append("Fields present on one side only:")
        for row in comparison["unpaired_fields"]:
            lines.append(f"  {row['name']} ({row['present_in']})")

    lines.append("pthread size/alignment:")
    for row in comparison["pthread"]:
        mark = "ok" if row["match"] else "MISMATCH"
        lines.append(
            f"  {row['name']}: C {fmt_size_align(row['c'])} vs "
            f"Rust {fmt_size_align(row['rust'])} [{mark}]"
        )

    summary = comparison["summary"]
    lines.append("")
    lines.append(
        "Summary: layout {matched}/{compared} matched, {mismatched} mismatched; "
        "fields {field_mismatched}/{field_compared} offset mismatches".format(
            matched=summary["layout_matched"],
            compared=summary["layout_compared"],
            mismatched=summary["layout_mismatched"],
            field_mismatched=summary["fields_mismatched"],
            field_compared=summary["fields_compared"],
        )
    )
    lines.append(
        "Rust values above are source-derived, not measured; no Rust target code "
        "was compiled or executed."
    )
    return "\n".join(lines) + "\n"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--def", dest="def_file", required=True, help="abi_items.def path"
    )
    parser.add_argument("--bin", required=True, help="dumped .switch_abi path")
    parser.add_argument("--env", default="", help="container env key=value file")
    parser.add_argument("--image", default="", help="host image key=value file")
    parser.add_argument("--libc-src", default="", help="libc-0.2.189 source root")
    parser.add_argument("--json", required=True, help="JSON output path")
    parser.add_argument("--report", required=True, help="human report output path")
    args = parser.parse_args(argv)

    items = parse_def(args.def_file)
    c_table = decode_table(args.bin, items)
    env = read_kv(args.env)
    image = read_kv(args.image)
    model, rust_info = load_rust_model(args.libc_src or None)
    comparison = compare(c_table, model) if model else None

    document = {
        "schema": 1,
        "generated_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "measured": {
            "method": (
                "devkitA64 GCC compile-only object; uint64 records in .switch_abi; "
                "objcopy --dump-section; little-endian decode"
            ),
            "image": image,
            "environment": env,
            "table": c_table,
        },
        "rust_source_derived": rust_info,
        "comparison": comparison,
    }

    with open(args.json, "w", encoding="utf-8") as handle:
        json.dump(document, handle, indent=2, sort_keys=False)
        handle.write("\n")

    report = build_report(c_table, comparison, rust_info, env, image)
    with open(args.report, "w", encoding="utf-8") as handle:
        handle.write(report)

    if comparison:
        summary = comparison["summary"]
        print(
            "ABI audit: layout {}/{} matched, {} mismatched; field offsets {}/{} "
            "mismatched".format(
                summary["layout_matched"],
                summary["layout_compared"],
                summary["layout_mismatched"],
                summary["fields_mismatched"],
                summary["fields_compared"],
            ),
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except (AuditError, OSError) as error:
        print(f"ABI audit failed: {error}", file=sys.stderr)
        raise SystemExit(1)
