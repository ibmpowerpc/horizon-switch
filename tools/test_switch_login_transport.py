#!/usr/bin/env python3
"""Independent loopback server for C TCP worker linked to real Rust login codec."""
import argparse
import concurrent.futures
import hashlib
from pathlib import Path
import socket
import subprocess
import threading
import time
import zlib


def vi(value):
    value &= 0xffffffff
    out = bytearray()
    while value > 127:
        out.append((value & 127) | 128)
        value >>= 7
    return bytes(out + bytes([value]))


def frame(body):
    return vi(len(body)) + body


def string(value):
    return vi(len(value)) + value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runner", type=Path)
    args = parser.parse_args()
    runner = str(args.runner.resolve())
    name = b"CraftRustNX"
    uuid = bytearray(hashlib.md5(b"OfflinePlayer:" + name).digest())
    uuid[6] = (uuid[6] & 15) | 48
    uuid[8] = (uuid[8] & 63) | 128
    uuid = bytes(uuid)
    session = bytes(range(16))
    success = b"\x02" + uuid + string(name) + b"\x00" + session
    compression = frame(b"\x03\x01")  # threshold=1
    compressed = frame(vi(len(success)) + zlib.compress(success))

    def fixture(label, chunks, stage, error=0, mode="normal", frames=None, threshold=None):
        stop = threading.Event()
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            listener.settimeout(3)
            port = listener.getsockname()[1]
            # Construct expected request independently of the Rust implementation.
            request = frame(b"\x00" + vi(777) + string(b"127.0.0.1") + port.to_bytes(2, "big") + b"\x02")
            request += frame(b"\x00" + string(name) + uuid)

            def serve():
                with listener.accept()[0] as conn:
                    conn.settimeout(13)
                    data = bytearray()
                    while len(data) < len(request):
                        part = conn.recv(len(request) - len(data))
                        assert part, "EOF before complete request"
                        data.extend(part)
                    assert bytes(data) == request, (label, data.hex())
                    for chunk in chunks:
                        conn.sendall(chunk)
                        if len(chunks) > 1:
                            stop.wait(0.02)
                    if label == "partial EOF":
                        conn.shutdown(socket.SHUT_WR)
                    # The diagnostic must close without sending LoginAcknowledged.
                    try:
                        assert conn.recv(1) == b"", "unexpected acknowledgement/data"
                    except ConnectionResetError:
                        pass  # Closing on malformed input may discard unread bytes.

            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                server = pool.submit(serve)
                started = time.monotonic()
                try:
                    result = subprocess.run([runner, "127.0.0.1", str(port), mode],
                                            text=True, capture_output=True, check=True, timeout=14)
                finally:
                    stop.set()
                elapsed = time.monotonic() - started
                server.result(timeout=3)
            lines = result.stdout.splitlines()
            values = list(map(int, lines[0].split()))
            assert values[:2] == [stage, error], (label, values)
            if frames is not None:
                assert values[7] == frames, (label, values)
            if threshold is not None:
                assert values[5] == threshold, (label, values)
            if stage == 5:  # LJ_ACCEPTED
                assert lines[1:4] == [uuid.hex(), session.hex(), name.decode()], (label, lines)
            if mode == "cancel-live":
                assert elapsed < 2, (label, elapsed)
            print(f"PASS {label} ({elapsed:.2f}s)", flush=True)

    fixture("raw success", [frame(success)], 5, frames=1, threshold=-1)
    fixture("coalesced compression and success", [compression + compressed], 5, frames=2, threshold=1)
    fixture("fragmented compressed stream", [bytes([b]) for b in compression + compressed], 5, frames=2)
    fixture("uncompressed envelope", [frame(b"\x03" + vi(256)) + frame(b"\x00" + success)], 5, frames=2, threshold=256)
    fixture("disabled compression", [frame(b"\x03" + vi(-1)) + frame(success)], 5, frames=2, threshold=-1)
    fixture("disconnect reason", [frame(b"\x00" + string(b'{"text":"test rejection"}'))], 6, frames=1)
    fixture("encryption required", [frame(b"\x01\x00\x01x\x01y\x00")], 7, frames=1)
    fixture("custom query", [frame(b"\x04\x00" + string(b"test:channel"))], 8, frames=1)
    fixture("cookie request", [frame(b"\x05" + string(b"test:cookie"))], 8, frames=1)
    fixture("unknown packet", [frame(b"\x06")], 8)
    broken = bytearray(zlib.compress(success))
    broken[-1] ^= 1
    fixture("bad zlib checksum", [compression + frame(vi(len(success)) + broken)], 9, error=8)
    fixture("inflated length above cap", [compression + frame(vi(65537) + b"x")], 9, error=8)
    fixture("frame length above cap", [vi(65537)], 9, error=8)
    # After the first SetCompression all further control frames need the new
    # envelope too. A high threshold keeps these small packets uncompressed.
    set_high = b"\x03" + vi(256)
    fixture("event limit", [frame(set_high) + frame(b"\x00" + set_high) * 15], 9, error=8, frames=16)
    fixture("partial EOF", [frame(success)[:3]], 9, error=7)
    fixture("cancel during receive", [], 10, mode="cancel-live")
    fixture("receive deadline", [], 9, error=4)
    result = subprocess.run([runner, "invalid.example", "25565", "cancel-before"],
                            text=True, capture_output=True, check=True, timeout=2)
    assert list(map(int, result.stdout.splitlines()[0].split()))[:2] == [10, 0]
    print("PASS cancel before DNS")


if __name__ == "__main__":
    main()
