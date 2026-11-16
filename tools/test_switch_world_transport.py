#!/usr/bin/env python3
"""Independent synthetic Java 26.3 world-data wire oracle, loopback only."""
import argparse
import concurrent.futures
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import time

from test_switch_configuration import (CFB8Session, compress_packet, decompress_packet,
    frame, openssl, receive_frame, receive_frame_encrypted, string, take_bytes, vi)
from test_switch_play_session import NAME, PLAYER_UUID, SESSION, login_body, position_body, pose_ack


def named(tag, key, payload):
    return bytes([tag]) + len(key).to_bytes(2, "big") + key + payload


def dimension_nbt(height):
    return (b"\x0a" + named(3, b"min_y", struct.pack(">i", 0)) +
            named(3, b"height", struct.pack(">i", height)) +
            named(1, b"has_skylight", b"\x01") + b"\x00")


def registry(name, entries):
    return b"\x07" + string(name) + vi(len(entries)) + b"".join(
        string(key) + b"\x01" + value for key, value in entries)


def bitset(mask):
    raw = mask.to_bytes((mask.bit_length() + 7) // 8, "little")
    return vi(len(raw)) + raw


def lights(sections, bad=False):
    count = sections + 2; mask = (1 << count) - 1
    arrays = []
    for index in range(count):
        n = 2047 if bad and index == 0 else 2048
        arrays.append(vi(n) + b"\xff" * n)
    return bitset(mask) + bitset(0) + bitset(0) + bitset(mask) + vi(count) + b"".join(arrays) + b"\x00"


def packed(values, bits):
    per = 64 // bits
    words = []
    for at in range(0, len(values), per):
        word = sum(value << (bits * index) for index, value in enumerate(values[at:at + per]))
        words.append(struct.pack(">Q", word))
    return b"".join(words)


def container(blocks, mode, bad=False):
    n = 4096 if blocks else 64
    value = 257 if blocks and mode == "global" else 7 if blocks else 2
    if mode == "single":
        return b"\x00" + vi(value)
    if mode == "global":
        bits = 9 if blocks else 4
        return bytes([bits]) + packed([value] * n, bits)
    bits = 4 if blocks else 1
    values = [((i >> 4) & 1) if blocks else 1 for i in range(n)]
    if bad:
        values[0] = 2  # Outside two-entry palette.
    return bytes([bits]) + b"\x02\x00" + vi(value) + packed(values, bits)


def chunk(height=128, mode="single", fault=None):
    sections = height // 16
    bits = height.bit_length()  # ceil(log2(height+1)) for power-of-two heights.
    maps = packed([64] * 256, bits)
    if fault == "heightmap": maps = maps[:-8]
    hmap = b"\x01\x00" + vi(len(maps) // 8) + maps
    data = b"".join(struct.pack(">hh", 4096, 0) +
        container(True, mode, fault == "palette" and i == 0) + container(False, mode)
        for i in range(sections))
    x, z = (2, 3) if fault == "no-sample" else (0, -1)
    block_entities = b"\x01\x00" + struct.pack(">h", 63) + b"\x00\x00"
    return (b"\x2e" + struct.pack(">ii", x, z) + hmap + string(data) +
            block_entities + lights(sections, fault == "light"))


def main():
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument("runner", type=Path)
    parser.add_argument("--snapshot", action="store_true")
    args = parser.parse_args(); runner = str(args.runner.resolve())
    build = Path(__file__).resolve().parents[1] / "platform/switch/login-probe/tests/build"
    with tempfile.TemporaryDirectory(prefix="world-keys-", dir=build) as directory:
        private = Path(directory) / "synthetic.pem"
        openssl("genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:1024", "-out", private)
        public = openssl("pkey", "-in", private, "-pubout", "-outform", "DER")
        token = b"\x01\x02\x03\x04"

        def fixture(label, threshold=256, mode="single", fault=None, height=128):
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0)); listener.listen(1); listener.settimeout(4)
                port = listener.getsockname()[1]

                def serve():
                    with listener.accept()[0] as conn:
                        conn.settimeout(4)
                        assert receive_frame(conn) == (b"\x00" + vi(777) + string(b"127.0.0.1") + port.to_bytes(2, "big") + b"\x02")
                        assert receive_frame(conn) == b"\x00" + string(NAME) + PLAYER_UUID
                        conn.sendall(frame(b"\x01\x00" + string(public) + string(token) + b"\x01"))
                        key = receive_frame(conn); assert key[0] == 1
                        encrypted, at = take_bytes(key, 1); challenge, at = take_bytes(key, at)
                        assert at == len(key)
                        secret = openssl("pkeyutl", "-decrypt", "-inkey", private, "-pkeyopt", "rsa_padding_mode:pkcs1", data=encrypted)
                        assert openssl("pkeyutl", "-decrypt", "-inkey", private, "-pkeyopt", "rsa_padding_mode:pkcs1", data=challenge) == token
                        cipher = CFB8Session(secret)
                        def send(*bodies):
                            conn.sendall(cipher.encrypt(b"".join(compress_packet(b, threshold) for b in bodies)))
                        def recv():
                            return decompress_packet(receive_frame_encrypted(conn, cipher), threshold)
                        def closed():
                            try: assert conn.recv(1) == b"", "unexpected client packet"
                            except ConnectionResetError: pass
                        success = b"\x02" + PLAYER_UUID + string(NAME) + b"\x00" + SESSION
                        initial = frame(b"\x03" + vi(threshold)) if threshold >= 0 else b""
                        initial += compress_packet(success, threshold)
                        dim = dimension_nbt(17 if fault == "dimension" else height)
                        initial += compress_packet(registry(b"minecraft:dimension_type", [(b"minecraft:overworld", dim)]), threshold)
                        if fault != "missing-biomes":
                            initial += compress_packet(registry(b"minecraft:worldgen/biome", [(f"minecraft:b{i}".encode(), b"\x0a\x00") for i in range(3)]), threshold)
                        conn.sendall(cipher.encrypt(initial))
                        assert recv() == b"\x03"
                        assert recv() == b"\x02" + string(b"minecraft:brand") + string(NAME)
                        assert recv() == b"\x00\x05en_us\x02\x00\x01\x7f\x01\x00\x01\x00"
                        if fault == "dimension": closed(); return
                        send(b"\x03", login_body()); assert recv() == b"\x03"
                        send(position_body()); assert recv() == pose_ack()
                        send(b"\x0c", chunk(height, mode, fault))
                        if fault in ("missing-biomes", "heightmap", "palette", "light"):
                            closed(); return
                        send(b"\x0b\x01"); assert recv() == b"\x0b\x3f\x80\x00\x00"
                        if fault == "pose-stale":
                            pose = (13.5, 65, -8.25, 30, 10)
                            send(position_body(pose, 8)); assert recv() == pose_ack(pose, 8)
                        if fault == "light-update":
                            send(b"\x31\x00" + vi((-1) & 0xffffffff) + lights(height // 16))
                        keep = b"\x12\x34\x56\x78\x9a\xbc\xde\xf0"
                        send(b"\x2d" + keep); assert recv() == b"\x1c" + keep
                        if fault in ("no-sample", "pose-stale"): conn.shutdown(socket.SHUT_WR)
                        closed()

                with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                    server = pool.submit(serve); started = time.monotonic()
                    result = subprocess.run([runner, "127.0.0.1", str(port), "normal"], capture_output=True, text=True, timeout=10)
                    assert result.returncode == 0 and not result.stderr, (label, result.returncode, result.stderr)
                    try: server.result(timeout=5)
                    except Exception as error: raise AssertionError((label, result.stdout, result.stderr)) from error
            lines = result.stdout.splitlines(); v = list(map(int, lines[0].split()))
            expected = (8, 0, -5) if fault == "missing-biomes" else (9, 7, 0) if fault in ("no-sample", "pose-stale") else (9, 8, -3) if fault in ("dimension", "heightmap", "palette", "light") else (19 if args.snapshot else 18, 0, 0)
            assert (v[0], v[1], v[37]) == expected, (label, v)
            column = None
            if args.snapshot:
                assert lines[1].startswith("COLUMN ")
                column = list(map(int, lines.pop(1).split()[1:]))
            assert lines[1:4] == [PLAYER_UUID.hex(), SESSION.hex(), NAME.decode()]
            assert v[38] == (0 if fault == "dimension" else 1)
            if expected[0] in (18, 19):
                assert v[39] == 3 and v[40] == 1 and v[42:45] == [height // 16, 1, 1], (label, v)
                assert v[45] == (height // 16 + 2) * (2 if fault == "light-update" else 1), (label, v)
                assert v[41] == (1 if fault == "light-update" else 0)
                assert v[47:51] == [1, 12, 63, -9]
                assert v[51:56] == [257 if mode == "global" else 7, 2, 0, height, 1], (label, v)
            if column is not None and expected[0] == 19:
                sections = height // 16
                def digest(values):
                    h = 2166136261
                    for value in values: h = ((h ^ value) * 16777619) & 0xffffffff
                    return h
                states = (257 if mode == "global" else (7 if mode == "single" or (i >> 4) & 1 else 0)
                          for _ in range(sections) for i in range(4096))
                assert column == [0, sections, sections * 4096, sections * 64, 0, -1, 0, 1,
                                  digest(states), digest([2] * (sections * 64))], (label, column)
            if fault in ("no-sample", "pose-stale"):
                assert v[47] == 0
                if column is not None: assert column[1:4] == [0, 0, 0]
            print(f"PASS {label} ({time.monotonic()-started:.2f}s)", flush=True)

        fixture("single palettes and sample")
        fixture("compressed replies", threshold=0)
        fixture("raw encrypted frames", threshold=-1)
        fixture("indirect palettes", mode="indirect")
        fixture("global palettes", mode="global")
        fixture("multiword light mask / large body", height=1024)
        for fault in ("dimension", "missing-biomes", "heightmap", "palette", "light", "no-sample", "pose-stale", "light-update"):
            fixture(fault, mode="indirect" if fault == "palette" else "single", fault=fault)


if __name__ == "__main__": main()
