#!/usr/bin/env python3
"""Decode the Rust-compiled .switch_abi table and compare it with the measured
C table in tools/switch-abi-audit/build/abi.json.

Only compiled values are compared. The existing source-derived Rust model in
decode.py is deliberately not used. Items without a public Rust field are
reported as unmeasured/missing instead of being assigned a value.
"""

import argparse
import datetime
import json
import pathlib
import struct
import sys

MAGIC = 0x314942412D484357
SCHEMA = 1
KIND_SIZE = 1
KIND_OFFSET = 2
KIND_ABSENT = 3
ABSENT_VALUE = 0xFFFFFFFFFFFFFFFF

ABSENT_REASONS = {
    ("sockaddr", "sa_len"): "C-only field; libc::sockaddr has no sa_len",
    ("sockaddr_in", "sin_len"): "C-only field; libc::sockaddr_in has no sin_len",
    ("sockaddr_storage", "ss_len"): "C-only field; libc::sockaddr_storage starts at ss_family",
    ("sockaddr_storage", "__ss_pad1"): "C-only field; Rust uses private __ss_padding",
    ("sockaddr_storage", "__ss_align"): "C-only field; no public Rust equivalent",
    ("sockaddr_storage", "__ss_pad2"): "C-only field; no public Rust equivalent",
}


def normalize(name):
    return name[7:] if name.startswith("struct ") else name


def parse_def(path):
    items = []
    for lineno, raw in enumerate(path.read_text().splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        if line.startswith("TYPE(") and line.endswith(")"):
            name = line[5:-1].strip()
            items.append({"kind": KIND_SIZE, "type": normalize(name), "line": lineno})
        elif line.startswith("FIELD(") and line.endswith(")"):
            struct_name, field_name = [normalize(p.strip()) for p in line[6:-1].split(",", 1)]
            items.append(
                {
                    "kind": KIND_OFFSET,
                    "struct": struct_name,
                    "field": field_name,
                    "line": lineno,
                }
            )
        else:
            raise SystemExit(f"Unsupported line {lineno} in {path}: {raw!r}")
    return items


def read_elf_machine(obj_path):
    data = obj_path.read_bytes()
    if data[:4] != b"\x7fELF":
        raise SystemExit(f"Not an ELF file: {obj_path}")
    elf_class, elf_data = data[4], data[5]
    endian = "<" if elf_data == 1 else ">"
    (machine,) = struct.unpack_from(endian + "H", data, 0x12)
    return {
        "path": str(obj_path),
        "elf_class": 64 if elf_class == 2 else 32,
        "little_endian": elf_data == 1,
        "elf_machine": machine,
        "elf_machine_name": {183: "AArch64"}.get(machine, f"unknown-{machine}"),
    }


def decode_table(bin_path, expected_count):
    blob = bin_path.read_bytes()
    if len(blob) % 8:
        raise SystemExit(f"Table size {len(blob)} is not a multiple of 8")
    words = list(struct.unpack("<" + "Q" * (len(blob) // 8), blob))
    if len(words) < 3:
        raise SystemExit("Table is too short")
    magic, schema, count = words[:3]
    if magic != MAGIC:
        raise SystemExit(f"Bad magic 0x{magic:016x}")
    if schema != SCHEMA:
        raise SystemExit(f"Unexpected schema {schema}")
    if count != expected_count:
        raise SystemExit(f"Record count {count} != def count {expected_count}")
    records = words[3:]
    if len(records) != count * 3:
        raise SystemExit(f"Record payload {len(records)} != {count * 3}")
    return {
        "magic": f"0x{magic:016x}",
        "schema": schema,
        "record_count": count,
        "bytes": len(blob),
        "records": [records[i : i + 3] for i in range(0, len(records), 3)],
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--def", required=True, type=pathlib.Path)
    ap.add_argument("--bin", required=True, type=pathlib.Path)
    ap.add_argument("--obj", required=True, type=pathlib.Path)
    ap.add_argument("--c-json", required=True, type=pathlib.Path)
    ap.add_argument("--env", type=pathlib.Path)
    ap.add_argument("--json", required=True, type=pathlib.Path)
    ap.add_argument("--report", required=True, type=pathlib.Path)
    ap.add_argument(
        "--strict",
        action="store_true",
        help=(
            "Exit non-zero if any compared size/alignment/offset still differs. "
            "Unmeasured items stay allowed but are listed; nothing is turned into "
            "a passing value."
        ),
    )
    args = ap.parse_args()

    items = parse_def(getattr(args, "def"))
    table = decode_table(args.bin, len(items))
    record_iter = iter(table["records"])

    rust_types, rust_fields, rust_absent = {}, {}, {}
    for item in items:
        kind, a, b = next(record_iter)
        if kind == KIND_ABSENT:
            if item["kind"] == KIND_SIZE:
                raise SystemExit(f"Type {item['type']} unexpectedly marked absent")
            key = (item["struct"], item["field"])
            rust_absent[key] = {
                "def_line": item["line"],
                "reason": ABSENT_REASONS.get(key, "no public Rust field"),
            }
            continue
        if item["kind"] == KIND_SIZE:
            if kind != KIND_SIZE:
                raise SystemExit(f"Expected size record for {item['type']}")
            rust_types[item["type"]] = {"size": a, "align": b, "def_line": item["line"]}
        else:
            if kind != KIND_OFFSET:
                raise SystemExit(f"Expected offset record for {item['struct']}.{item['field']}")
            rust_fields[(item["struct"], item["field"])] = {
                "offset": a,
                "def_line": item["line"],
            }

    c = json.loads(args.c_json.read_text())
    measured = c.get("measured")
    if not measured or "table" not in measured:
        raise SystemExit(f"{args.c_json} has no measured.table")
    c_types = measured["table"]["types"]
    c_structs = measured["table"]["structs"]

    comparison = {"type_size_align": {}, "field_offset": {}, "unmeasured": {}}
    mismatches = []

    for name, cval in c_types.items():
        rval = rust_types.get(name)
        if rval is None:
            comparison["type_size_align"][name] = {"c": [cval["size"], cval["align"]], "rust": None, "status": "missing"}
            mismatches.append({"item": name, "field": None, "kind": "type", "c": [cval["size"], cval["align"]], "rust": None})
            continue
        c_pair = [cval["size"], cval["align"]]
        r_pair = [rval["size"], rval["align"]]
        ok = c_pair == r_pair
        comparison["type_size_align"][name] = {"c": c_pair, "rust": r_pair, "status": "match" if ok else "mismatch"}
        if not ok:
            mismatches.append({"item": name, "field": None, "kind": "type", "c": c_pair, "rust": r_pair})

    for name, cval in c_structs.items():
        rval = rust_types.get(name)
        if rval is None:
            comparison["type_size_align"][name] = {"c": [cval["size"], cval["align"]], "rust": None, "status": "missing"}
            mismatches.append({"item": name, "field": None, "kind": "struct", "c": [cval["size"], cval["align"]], "rust": None})
            continue
        c_pair = [cval["size"], cval["align"]]
        r_pair = [rval["size"], rval["align"]]
        ok = c_pair == r_pair
        comparison["type_size_align"][name] = {"c": c_pair, "rust": r_pair, "status": "match" if ok else "mismatch"}
        if not ok:
            mismatches.append({"item": name, "field": None, "kind": "struct", "c": c_pair, "rust": r_pair})
        for field, fval in cval.get("fields", {}).items():
            key = (name, field)
            rfield = rust_fields.get(key)
            if rfield is None:
                reason = rust_absent.get(key, {}).get("reason", "not measured")
                comparison["field_offset"][f"{name}.{field}"] = {"c": fval["offset"], "rust": None, "status": "unmeasured", "reason": reason}
                comparison["unmeasured"][f"{name}.{field}"] = reason
                continue
            ok = fval["offset"] == rfield["offset"]
            comparison["field_offset"][f"{name}.{field}"] = {"c": fval["offset"], "rust": rfield["offset"], "status": "match" if ok else "mismatch"}
            if not ok:
                mismatches.append({"item": name, "field": field, "kind": "field", "c": fval["offset"], "rust": rfield["offset"]})

    summary = {
        "types_compared": len(c_types),
        "structs_compared": len(c_structs),
        "type_size_align_total": len(comparison["type_size_align"]),
        "type_size_align_matches": sum(1 for v in comparison["type_size_align"].values() if v["status"] == "match"),
        "field_offsets_total": sum(1 for v in comparison["field_offset"].values() if v["status"] != "unmeasured"),
        "field_offsets_matches": sum(1 for v in comparison["field_offset"].values() if v["status"] == "match"),
        "mismatch_count": len(mismatches),
        "unmeasured_count": len(comparison["unmeasured"]),
    }

    elf = read_elf_machine(args.obj)
    if elf["elf_machine_name"] != "AArch64":
        raise SystemExit(f"Object is not AArch64: {elf}")

    toolchain = {}
    if args.env:
        for line in args.env.read_text().splitlines():
            key, sep, value = line.partition("=")
            if sep:
                toolchain[key] = value

    patched = toolchain.get("libc_patched", "false")
    mode = "patched-libc" if patched == "true" else "unpatched-libc"

    out = {
        "schema": 2,
        "generated_utc": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "method": "Rust compile-only staticlib member, .switch_abi, devkitA64 objcopy; values are compiler-evaluated size_of/align_of/offset_of!",
        "mode": mode,
        "toolchain": toolchain,
        "source_under_test": {
            "libc": "0.2.189 (patched local Switch copy)" if patched == "true" else "0.2.189 (unpatched registry source)",
            "rust_target": "aarch64-nintendo-switch-freestanding",
            "extra_cfg": ["unix", 'target_family="unix"', 'target_env="newlib"'],
        },
        "object": elf,
        "table": {
            "magic": table["magic"],
            "schema": table["schema"],
            "record_count": table["record_count"],
            "bytes": table["bytes"],
            "types": rust_types,
            "fields": {f"{s}.{f}": v for (s, f), v in rust_fields.items()},
            "absent": {f"{s}.{f}": v for (s, f), v in rust_absent.items()},
        },
        "comparison": {
            "c_measured": str(args.c_json),
            "c_method": measured.get("method", ""),
            **comparison,
            "mismatches": mismatches,
            "summary": summary,
        },
    }

    args.json.write_text(json.dumps(out, indent=2) + "\n")

    lines = [
        f"Rust-compiled libc 0.2.189 ABI vs measured Switch C ABI ({mode})",
        f"Object: {elf['path']} ({elf['elf_machine_name']}, {elf['elf_class']}-bit)",
        f"Table: {table['record_count']} records, {table['bytes']} bytes ({table['magic']})",
        "",
        "Measured Rust types (size/align):",
        "  " + "  ".join(f"{n}={v['size']}/{v['align']}" for n, v in rust_types.items()),
        "",
        "Size/alignment mismatches:",
    ]
    type_bad = [m for m in mismatches if m["kind"] in ("type", "struct")]
    if type_bad:
        for m in type_bad:
            lines.append(f"  {m['item']}: C {m['c'][0]}/{m['c'][1]} vs Rust {m['rust'][0]}/{m['rust'][1]}")
    else:
        lines.append("  none")
    lines.append("")
    lines.append("Field offset mismatches:")
    field_bad = [m for m in mismatches if m["kind"] == "field"]
    if field_bad:
        for m in field_bad:
            lines.append(f"  {m['item']}.{m['field']}: C {m['c']} vs Rust {m['rust']}")
    else:
        lines.append("  none")
    lines.append("")
    lines.append("Unmeasured (no public/represented Rust field; no value invented):")
    if comparison["unmeasured"]:
        for name, reason in comparison["unmeasured"].items():
            lines.append(f"  {name}: {reason}")
    else:
        lines.append("  none")
    lines.append("")
    lines.append(
        "Summary: type size/align {type_size_align_matches}/{type_size_align_total}; "
        "field offsets {field_offsets_matches}/{field_offsets_total}; "
        "{mismatch_count} mismatches; {unmeasured_count} unmeasured".format(**summary)
    )
    if args.strict:
        covered = summary["type_size_align_total"] + summary["field_offsets_total"]
        covered_ok = summary["type_size_align_matches"] + summary["field_offsets_matches"]
        verdict = "PASS" if summary["mismatch_count"] == 0 else "FAIL"
        lines.append(f"Strict verification ({mode}): {verdict} - {covered_ok}/{covered} covered checks match")
        if summary["mismatch_count"] == 0 and summary["unmeasured_count"]:
            lines.append(
                f"Note: {summary['unmeasured_count']} item(s) remain explicitly unmeasured; "
                "they are not counted as matches."
            )
    args.report.write_text("\n".join(lines) + "\n")
    print("\n".join(lines))

    if args.strict and summary["mismatch_count"]:
        print(
            f"\nSTRICT FAILURE: {summary['mismatch_count']} covered size/align/offset mismatch(es) remain.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
