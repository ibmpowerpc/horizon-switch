#!/usr/bin/env python3
"""Check a probe's packaged NRO/NACP and writable ELF dynamic relocations."""
import argparse
import hashlib
import json
from pathlib import Path
import struct


def verify(nro_path, elf_path, title, version):
    nro = nro_path.read_bytes()
    assert nro[0x10:0x14] == b"NRO0", "missing NRO0 magic"
    asset = struct.unpack_from("<I", nro, 0x18)[0]
    assert nro[asset:asset + 4] == b"ASET", "missing asset header"
    offset, size = struct.unpack_from("<QQ", nro, asset + 24)
    nacp = nro[asset + offset:asset + offset + size]
    assert len(nacp) == size == 0x4000, "truncated or unexpected NACP"
    actual_title = nacp[:0x200].split(b"\0", 1)[0].decode()
    actual_version = nacp[0x3060:0x3070].split(b"\0", 1)[0].decode()
    assert actual_title == title, (actual_title, title)
    assert actual_version == version, (actual_version, version)

    elf = elf_path.read_bytes()
    assert elf[:6] == b"\x7fELF\x02\x01", "expected ELF64 little-endian"
    assert struct.unpack_from("<HH", elf, 16) == (3, 183), "expected AArch64 DYN"
    phoff, shoff = struct.unpack_from("<QQ", elf, 32)
    phsize, phnum, shsize, shnum = struct.unpack_from("<4H", elf, 54)
    assert phsize == 56 and shsize == 64, "unexpected table entry size"
    writable = []
    dynamic = []
    for i in range(phnum):
        kind, flags, offset, vaddr, _, filesz, memsz, _ = struct.unpack_from(
            "<II6Q", elf, phoff + i * phsize)
        if kind == 1 and flags & 2:
            writable.append((vaddr, vaddr + memsz))
        if kind == 2:
            dynamic.extend(struct.iter_unpack("<qQ", elf[offset:offset + filesz]))
    assert dynamic, "missing dynamic table"
    assert not any(tag == 22 or tag == 30 and value & 4 for tag, value in dynamic), "DT_TEXTREL"
    targets = []
    for i in range(shnum):
        _, kind, flags, _, offset, size, _, _, _, entsize = struct.unpack_from(
            "<II4QII2Q", elf, shoff + i * shsize)
        if not flags & 2:  # Only runtime sections, not debug relocations.
            continue
        if kind == 8:  # SHT_NOBITS (BSS) occupies memory, not bytes in the ELF.
            continue
        data = elf[offset:offset + size]
        assert len(data) == size, "truncated section"
        if kind == 4:  # SHT_RELA
            assert entsize == 24 and size % entsize == 0
            targets.extend(item[0] for item in struct.iter_unpack("<QQq", data))
        elif kind == 19:  # SHT_RELR
            assert entsize == 8 and size % entsize == 0
            cursor = None
            for (word,) in struct.iter_unpack("<Q", data):
                if not word & 1:
                    targets.append(word)
                    cursor = word + 8
                else:
                    assert cursor is not None, "RELR bitmap without base"
                    targets.extend(cursor + 8 * bit for bit in range(63) if word >> (bit + 1) & 1)
                    cursor += 63 * 8
    assert targets, "no dynamic relocations checked"
    assert all(any(lo <= addr and addr + 8 <= hi for lo, hi in writable)
               for addr in targets), "dynamic relocation writes a non-writable segment"
    return dict(bytes=len(nro), sha256=hashlib.sha256(nro).hexdigest(), magic="NRO0",
                elf="ELF64 LE AArch64 DYN", textrel=False,
                dynamic_relocations_in_writable_segments=len(targets),
                title=actual_title, version=actual_version)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("nro", type=Path)
    parser.add_argument("elf", type=Path)
    parser.add_argument("--title", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = json.dumps(verify(args.nro, args.elf, args.title, args.version), indent=2) + "\n"
    if args.output:
        args.output.write_text(result)
    print(result, end="")


if __name__ == "__main__":
    main()
