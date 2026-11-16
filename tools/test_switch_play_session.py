#!/usr/bin/env python3
"""Independent encrypted loopback oracle for bounded native Switch play sessions."""
import argparse
import concurrent.futures
import hashlib
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import threading
import time

from test_switch_configuration import (
    CFB8Session, compress_packet, decompress_packet, frame, openssl,
    receive_frame, receive_frame_encrypted, string, take_bytes, vi,
)

NAME = b"CraftRustNX"
PLAYER_UUID = bytearray(hashlib.md5(b"OfflinePlayer:" + NAME).digest())
PLAYER_UUID[6] = (PLAYER_UUID[6] & 15) | 48
PLAYER_UUID[8] = (PLAYER_UUID[8] & 63) | 128
PLAYER_UUID = bytes(PLAYER_UUID)
SESSION = bytes(range(16))
DIMENSION = b"minecraft:overworld"
POSITION = (12.5, 64.0, -8.25, 30.0, 10.0)


def spawn_info():
    # Dimension type, name, seed, game type, optional previous type,
    # debug/flat/optional last-death, portal cooldown, sea level.
    return (b"\x00" + string(DIMENSION) + struct.pack(">q", 123456789) +
            b"\x01\x00\x00\x00\x00\x00\x3f")


def login_body():
    return (b"\x32" + struct.pack(">i", 42) + b"\x00\x01" + string(DIMENSION) +
            b"\x14\x02\x02\x00\x01\x00" + spawn_info() + b"\x00\x00")


def position_body(pose=POSITION, teleport=7, relatives=0):
    x, y, z, yaw, pitch = pose
    return b"\x49" + vi(teleport) + struct.pack(">6d2fi", x, y, z, 0, 0, 0, yaw, pitch, relatives)


def pose_ack(pose=POSITION, teleport=7):
    return b"\x00" + vi(teleport) + struct.pack(">3d2f", *pose)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runner", type=Path)
    args = parser.parse_args()
    runner = str(args.runner.resolve())
    build = Path(__file__).resolve().parents[1] / "platform/switch/login-probe/tests/build"
    build.mkdir(exist_ok=True, parents=True)
    with tempfile.TemporaryDirectory(prefix="play-keys-", dir=build) as directory:
        private = Path(directory) / "synthetic.pem"
        openssl("genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:1024", "-out", private)
        public = openssl("pkey", "-in", private, "-pubout", "-outform", "DER")
        token = b"\x01\x02\x03\x04"

        def fixture(label, scenario="success", threshold=256, fragmented=False, bundled=False):
            stop = threading.Event()
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                listener.listen(1)
                listener.settimeout(4)
                port = listener.getsockname()[1]

                def serve():
                    with listener.accept()[0] as conn:
                        conn.settimeout(4)
                        assert receive_frame(conn) == (b"\x00" + vi(777) + string(b"127.0.0.1") +
                                                       port.to_bytes(2, "big") + b"\x02")
                        assert receive_frame(conn) == b"\x00" + string(NAME) + PLAYER_UUID
                        conn.sendall(frame(b"\x01\x00" + string(public) + string(token) + b"\x01"))
                        key = receive_frame(conn)
                        assert key[0] == 1
                        encrypted_secret, at = take_bytes(key, 1)
                        encrypted_token, at = take_bytes(key, at)
                        assert at == len(key)
                        secret = openssl("pkeyutl", "-decrypt", "-inkey", private,
                                         "-pkeyopt", "rsa_padding_mode:pkcs1", data=encrypted_secret)
                        assert openssl("pkeyutl", "-decrypt", "-inkey", private,
                                       "-pkeyopt", "rsa_padding_mode:pkcs1", data=encrypted_token) == token
                        assert len(secret) == 16
                        cipher = CFB8Session(secret)

                        def send(*bodies):
                            ciphertext = cipher.encrypt(b"".join(compress_packet(b, threshold) for b in bodies))
                            if fragmented:
                                for start in range(0, len(ciphertext), 3):
                                    conn.sendall(ciphertext[start:start + 3])
                                    stop.wait(0.001)
                            else:
                                conn.sendall(ciphertext)

                        def recv():
                            return decompress_packet(receive_frame_encrypted(conn, cipher), threshold)

                        def closed():
                            try:
                                assert conn.recv(1) == b"", "unexpected client data after terminal result"
                            except ConnectionResetError:
                                pass

                        success = b"\x02" + PLAYER_UUID + string(NAME) + b"\x00" + SESSION
                        initial = frame(b"\x03" + vi(threshold)) if threshold >= 0 else b""
                        # LoginFinished and configuration metadata share one continuous stream.
                        initial += compress_packet(success, threshold) + compress_packet(b"\x07opaque registry", threshold)
                        conn.sendall(cipher.encrypt(initial))
                        assert recv() == b"\x03"
                        assert recv() == b"\x02" + string(b"minecraft:brand") + string(NAME)
                        assert recv() == b"\x00\x05en_us\x02\x00\x01\x7f\x01\x00\x01\x00"
                        if scenario == "cancel":
                            send(b"\x03")
                            assert recv() == b"\x03"
                            closed()
                            return
                        # FinishConfiguration and first play Login are coalesced.
                        first = position_body() if scenario == "position-before-login" else login_body()
                        send(b"\x03", first)
                        assert recv() == b"\x03"
                        if scenario == "position-before-login":
                            closed()
                            return
                        if scenario == "duplicate-login":
                            send(login_body())
                            closed()
                            return
                        if scenario == "relative-first":
                            send(position_body(relatives=1))
                            closed()
                            return
                        if scenario == "nonfinite":
                            send(position_body((float("nan"), 64, 0, 0, 0)))
                            closed()
                            return
                        send(position_body())
                        assert recv() == pose_ack(), "teleport ACK must include id and full pose"
                        if scenario == "relative":
                            send(position_body((1.5, 2.0, -0.75, 5.0, 100.0), 8, 31))
                            assert recv() == pose_ack((14, 66, -9, 35, 90), 8), "relative pose or pitch clamp wrong"
                        if scenario in ("reconfigure", "respawn", "transfer", "malformed", "unknown", "disconnect", "pack"):
                            if scenario == "reconfigure":
                                send(b"\x78")
                            elif scenario == "respawn":
                                send(b"\x54" + spawn_info() + b"\x00")
                            elif scenario == "transfer":
                                send(vi(132) + string(b"other.server.test") + vi(25565))
                            elif scenario == "malformed":
                                send(b"\x2d\x01")  # Complete frame, incomplete Long.
                            elif scenario == "unknown":
                                send(vi(144) + b"future")
                            else:
                                if scenario == "pack":
                                    pack_id = bytes(range(16))
                                    send(b"\x52" + pack_id + string(b"https://example.com/pack.zip") +
                                         string(b"") + b"\x01\x00")
                                    assert recv() == b"\x32" + pack_id + b"\x01"
                                reason = b"Session ended by test server"
                                send(b"\x20\x08" + len(reason).to_bytes(2, "big") + reason)
                            closed()
                            return
                        # Reply routing must use PLAY IDs, not configuration IDs.
                        send(b"\x3e\x12\x34\x56\x78", b"\x15" + string(b"minecraft:test_cookie"))
                        assert recv() == b"\x2d\x12\x34\x56\x78"
                        assert recv() == b"\x15" + string(b"minecraft:test_cookie") + b"\x00"
                        if bundled:
                            send(b"\x00")
                        if scenario != "no-batch":
                            send(b"\x0c")
                        send(b"\x2e" + struct.pack(">ii", -2, 3) + b"\x00" * 70000)
                        if scenario == "batch-mismatch":
                            send(b"\x0b\x02")
                            closed()
                            return
                        if scenario == "nested-batch":
                            send(b"\x0c")
                            closed()
                            return
                        keepalive = bytes.fromhex("123456789abcdef0")
                        send(b"\x2d" + keepalive)
                        assert recv() == b"\x1c" + keepalive
                        if scenario == "no-batch":
                            conn.shutdown(socket.SHUT_WR)
                            closed()
                            return
                        # KeepAlive must not finish the diagnostic inside an open batch.
                        send(b"\x0b\x01")
                        assert recv() == b"\x0b\x3f\x80\x00\x00"
                        if bundled:
                            send(b"\x00")  # Must wait until this delimiter to close.
                        closed()

                with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                    server = pool.submit(serve)
                    started = time.monotonic()
                    try:
                        result = subprocess.run([runner, "127.0.0.1", str(port),
                                                 "cancel-play" if scenario == "cancel" else "normal"],
                                                capture_output=True, text=True, timeout=10)
                    finally:
                        stop.set()
                    assert result.returncode == 0, (label, result.returncode, result.stderr)
                    try:
                        server.result(timeout=5)
                    except Exception as error:
                        raise AssertionError((label, result.stdout, result.stderr)) from error
            lines = result.stdout.splitlines()
            values = list(map(int, lines[0].split()))
            expected = {
                "relative-first": (8, 0), "position-before-login": (9, 8),
                "nonfinite": (9, 8), "duplicate-login": (9, 8),
                "reconfigure": (8, 0), "respawn": (8, 0), "transfer": (8, 0),
                "malformed": (9, 8), "unknown": (8, 0), "disconnect": (6, 0),
                "pack": (6, 0), "batch-mismatch": (9, 8), "nested-batch": (9, 8),
                "no-batch": (9, 7), "cancel": (10, 0),
            }.get(scenario, (17, 0))
            assert tuple(values[:2]) == expected, (label, values)
            assert not result.stderr, (label, result.stderr)
            assert values[8:10] == [1, 1] and values[12:14] == [1, 1], (label, values)
            assert lines[1:4] == [PLAYER_UUID.hex(), SESSION.hex(), NAME.decode()]
            if scenario != "cancel" and scenario != "position-before-login":
                assert values[30] == 1 and values[32] == 42 and values[33] == 1, (label, values)
                assert lines[4] == DIMENSION.decode()
            if expected[0] == 17:
                assert values[24:29] == [2 if scenario == "relative" else 1, 1, 1, 1, 1], (label, values)
                assert values[35:37] == [-2, 3], (label, values)
                pose = tuple(float.fromhex(x) for x in lines[5].split())
                assert pose == ((14, 66, -9, 35, 90) if scenario == "relative" else POSITION), (label, pose)
                assert values[23] > 70000
                if bundled:
                    assert values[29] == 0, "closing bundle delimiter was not consumed"
            if scenario == "no-batch":
                assert values[24] == values[25] == values[27] == 1 and values[28] == 0
            if scenario == "pack":
                assert values[19:21] == [1, 1]
            print(f"PASS {label} ({time.monotonic() - started:.2f}s)", flush=True)

        fixture("encrypted play session")
        fixture("zero threshold compresses every reply", threshold=0)
        fixture("encrypted raw framing", threshold=-1)
        fixture("fragmented controls and large chunk", fragmented=True)
        fixture("relative pose and pitch clamp", "relative")
        fixture("success waits for bundle end", bundled=True)
        for scenario in ("relative-first", "position-before-login", "duplicate-login", "nonfinite",
                         "batch-mismatch", "nested-batch", "no-batch", "reconfigure", "respawn",
                         "transfer", "malformed", "unknown", "disconnect", "pack", "cancel"):
            fixture(scenario, scenario)


if __name__ == "__main__":
    main()
