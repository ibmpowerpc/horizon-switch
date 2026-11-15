#!/usr/bin/env python3
"""Loopback RSA/CFB8 login oracle; keys are temporary test data, never game data."""
import argparse
import concurrent.futures
import hashlib
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import zlib

OPENSSL = "/opt/homebrew/opt/openssl@3/bin/openssl"


def vi(value):
    out = bytearray()
    while value > 127:
        out.append((value & 127) | 128)
        value >>= 7
    return bytes(out + bytes([value]))


def frame(body):
    return vi(len(body)) + body


def string(data):
    return vi(len(data)) + data


def openssl(*args, data=None):
    return subprocess.run([OPENSSL, *map(str, args)], input=data,
                          capture_output=True, check=True).stdout


def exact(conn, size):
    result = bytearray()
    while len(result) < size:
        chunk = conn.recv(size - len(result))
        assert chunk, "unexpected client EOF"
        result.extend(chunk)
    return bytes(result)


def receive_frame(conn):
    length = 0
    for i in range(5):
        byte = exact(conn, 1)[0]
        length |= (byte & 127) << (i * 7)
        if byte < 128:
            assert 0 < length < 2048
            return exact(conn, length)
    raise AssertionError("bad client frame prefix")


def take_bytes(body, pos):
    length = 0
    for i in range(5):
        byte = body[pos]
        pos += 1
        length |= (byte & 127) << (7 * i)
        if byte < 128:
            assert pos + length <= len(body)
            return body[pos:pos + length], pos + length
    raise AssertionError("bad client array length")


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
    build = Path(__file__).resolve().parents[1] / "platform/switch/login-probe/tests/build"
    build.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="synthetic-keys-", dir=build) as tmp:
        keys = {}
        for bits in (1024, 2048, 512):
            private = Path(tmp) / f"test-{bits}.pem"
            openssl("genpkey", "-algorithm", "RSA", "-pkeyopt", f"rsa_keygen_bits:{bits}", "-out", private)
            public = openssl("pkey", "-in", private, "-pubout", "-outform", "DER")
            keys[bits] = private, public

        def fixture(label, auth=0, bits=1024, outcome="accept", fragment=False, bad_key=None, long_token=False):
            stop = threading.Event()
            private, der = keys[bits]
            if bad_key == "trailing":
                der += b"\x00"
            elif bad_key == "malformed":
                der = b"\x30\x01\x00"
            token = b"\x11\x22\x33\x44" if not long_token else b"a" * 118
            hello = b"\x01\x00" + string(der) + string(token) + bytes([auth])
            crypto_failure = bad_key or bits == 512 or long_token
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                listener.listen(1)
                listener.settimeout(3)
                port = listener.getsockname()[1]
                hello_request = b"\x00" + vi(777) + string(b"127.0.0.1") + port.to_bytes(2, "big") + b"\x02"
                login_request = b"\x00" + string(name) + uuid

                def serve():
                    with listener.accept()[0] as conn:
                        conn.settimeout(3)
                        assert receive_frame(conn) == hello_request
                        assert receive_frame(conn) == login_request
                        conn.sendall(frame(hello))
                        if crypto_failure:
                            assert conn.recv(1) == b"", "client continued after invalid key/token"
                            return
                        key = receive_frame(conn)  # MUST be plaintext before cipher switch.
                        assert key[0] == 1
                        secret_cipher, pos = take_bytes(key, 1)
                        token_cipher, pos = take_bytes(key, pos)
                        assert pos == len(key) and len(secret_cipher) == len(token_cipher) == bits // 8
                        secret = openssl("pkeyutl", "-decrypt", "-inkey", private,
                                         "-pkeyopt", "rsa_padding_mode:pkcs1", data=secret_cipher)
                        echoed = openssl("pkeyutl", "-decrypt", "-inkey", private,
                                         "-pkeyopt", "rsa_padding_mode:pkcs1", data=token_cipher)
                        assert len(secret) == 16 and echoed == token
                        if outcome == "cancel":
                            conn.settimeout(3)
                            assert conn.recv(1) == b""
                            return
                        if outcome == "reject":
                            plaintext = frame(b"\x00" + string(b'{"text":"test server rejects offline profile"}'))
                        elif outcome == "repeat":
                            plaintext = frame(hello)
                        elif outcome == "partial":
                            plaintext = frame(success)[:3]
                        else:
                            compressed = bytearray(zlib.compress(success))
                            if outcome == "bad-checksum":
                                compressed[-1] ^= 1
                            plaintext = frame(b"\x03\x01") + frame(vi(len(success)) + compressed)
                        ciphertext = openssl("enc", "-aes-128-cfb8", "-K", secret.hex(),
                                             "-iv", secret.hex(), "-nopad", data=plaintext)
                        chunks = [bytes([b]) for b in ciphertext] if fragment else [ciphertext]
                        for chunk in chunks:
                            conn.sendall(chunk)
                            if fragment:
                                stop.wait(0.002)
                        if outcome == "partial":
                            conn.shutdown(socket.SHUT_WR)
                        try:
                            assert conn.recv(1) == b"", "diagnostic sent a login acknowledgement"
                        except ConnectionResetError:
                            pass

                with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                    server = pool.submit(serve)
                    started = time.monotonic()
                    try:
                        result = subprocess.run([runner, "127.0.0.1", str(port),
                                                 "cancel-live" if outcome == "cancel" else "normal"],
                                                capture_output=True, text=True, check=True, timeout=14)
                    finally:
                        stop.set()
                    server.result(timeout=4)
                    elapsed = time.monotonic() - started
                lines = result.stdout.splitlines()
                values = list(map(int, lines[0].split()))
                if crypto_failure:
                    expected = [9, 10]  # LJ_ERROR / LJ_ERR_CRYPTO
                    assert values[9] == 0
                elif outcome == "reject":
                    expected = [6, 0]
                elif outcome == "cancel":
                    expected = [10, 0]
                    assert elapsed < 2
                elif outcome == "partial":
                    expected = [9, 7]
                elif outcome in ("repeat", "bad-checksum"):
                    expected = [9, 8]
                else:
                    expected = [5, 0]
                    assert lines[1:4] == [uuid.hex(), session.hex(), name.decode()]
                    assert values[5] == 1 and values[7] == 3
                assert values[:2] == expected, (label, values)
                assert values[8] == auth, (label, values)
                if not crypto_failure:
                    assert values[9] == 1, (label, values)
                print(f"PASS {label} ({elapsed:.2f}s)", flush=True)

        fixture("encrypted profile / flag false")
        fixture("encrypted profile / flag true", auth=1)
        fixture("encrypted fragmented stream", auth=1, fragment=True)
        fixture("RSA 2048 encrypted profile", bits=2048)
        fixture("server refusal after key exchange", auth=1, outcome="reject")
        fixture("encrypted bad checksum", outcome="bad-checksum")
        fixture("repeated encryption request", outcome="repeat")
        fixture("partial encrypted EOF", outcome="partial")
        fixture("cancel after key exchange", outcome="cancel")
        fixture("malformed DER", bad_key="malformed")
        fixture("trailing DER", bad_key="trailing")
        fixture("unsupported RSA 512", bits=512)
        fixture("oversized token for RSA 1024", long_token=True)


if __name__ == "__main__":
    main()
