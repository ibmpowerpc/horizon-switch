#!/usr/bin/env python3
"""Loopback fixtures for actual POSIX transport + Rust C ABI; no game assets."""
import argparse
import concurrent.futures
from pathlib import Path
import socket
import subprocess
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runner", type=Path)
    args = parser.parse_args()
    runner = str(args.runner.resolve())
    # Independent wire vectors: protocol 777, host 127.0.0.1, status state.
    request_prefix = b"\x10\x00\x89\x06\x09127.0.0.1"
    response = b'\x04\x00\x02{}'

    def fixture(name, chunks, expected_stage, expected_error, mode="normal", hold=False):
        stop = threading.Event()
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            listener.settimeout(3)
            port = listener.getsockname()[1]
            expected_request = request_prefix + port.to_bytes(2, "big") + b"\x01\x01\x00"

            def serve():
                with listener.accept()[0] as conn:
                    conn.settimeout(3)
                    data = bytearray()
                    while len(data) < len(expected_request):
                        part = conn.recv(len(expected_request) - len(data))
                        assert part, "client closed before completing request"
                        data.extend(part)
                    assert bytes(data) == expected_request, bytes(data).hex()
                    for chunk in chunks:
                        conn.sendall(chunk)
                        # TCP can coalesce; bytewise codec prefix tests separately
                        # pin incremental parsing. This exercises real recv boundaries.
                        if len(chunks) > 1:
                            stop.wait(0.02)
                    if hold:
                        stop.wait(13)

            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                server = pool.submit(serve)
                started = time.monotonic()
                try:
                    result = subprocess.run([runner, "127.0.0.1", str(port), mode],
                                            text=True, capture_output=True, timeout=14, check=True)
                finally:
                    stop.set()
                elapsed = time.monotonic() - started
                server.result(timeout=3)
            lines = result.stdout.splitlines()
            values = list(map(int, lines[0].split()))
            assert values[:2] == [expected_stage, expected_error], (name, values)
            if expected_stage == 5:  # NP_DONE
                assert bytes.fromhex(lines[1]) == b"{}", (name, lines)
            if mode == "cancel-live":
                assert elapsed < 2, (name, elapsed)
            print(f"PASS {name} ({elapsed:.2f}s)")

    fixture("valid response", [response], 5, 0)
    fixture("response in pieces", [bytes([b]) for b in response], 5, 0)
    fixture("EOF before frame complete", [response[:3]], 6, 7)
    fixture("wrong packet id", [b"\x02\x01\x00"], 6, 8)
    fixture("oversized frame", [b"\xff\xff\xff\xff\x0f"], 6, 8)
    fixture("cancel while receiving", [], 7, 0, mode="cancel-live", hold=True)
    fixture("receive deadline", [], 6, 4, hold=True)
    result = subprocess.run([runner, "invalid.example", "25565", "cancel-before"],
                            text=True, capture_output=True, check=True, timeout=2)
    assert list(map(int, result.stdout.splitlines()[0].split()))[:2] == [7, 0]
    print("PASS cancel before DNS")


if __name__ == "__main__":
    main()
