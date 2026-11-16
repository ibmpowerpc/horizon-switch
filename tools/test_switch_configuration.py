#!/usr/bin/env python3
"""Loopback CONFIGURATION integration fixture for native Switch Java26.3 diagnostic.
Independent encrypted LOGIN→CONFIGURATION→first PLAY frame oracle. Tests honest
protocol, encrypted continuity, fragmented/coalesced receives, compression threshold
settings, >64KiB registry, control replies, malformed protocol, unknown packets,
action-required consent, transfer rejection, required pack decline, cancellation."""
import argparse
import concurrent.futures
import ctypes
import ctypes.util
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
    """VarInt encoding."""
    out = bytearray()
    while value > 127:
        out.append((value & 127) | 128)
        value >>= 7
    return bytes(out + bytes([value]))


def frame(body):
    """Framed packet."""
    return vi(len(body)) + body


def string(data):
    """String with length prefix."""
    return vi(len(data)) + data


def uuid_bytes(uuid_hex):
    """16-byte UUID from hex string."""
    return bytes.fromhex(uuid_hex.replace("-", ""))


def openssl(*args, data=None):
    """Run openssl command."""
    return subprocess.run([OPENSSL, *map(str, args)], input=data,
                          capture_output=True, check=True).stdout


def exact(conn, size):
    """Read exactly size bytes."""
    result = bytearray()
    while len(result) < size:
        chunk = conn.recv(size - len(result))
        assert chunk, "unexpected client EOF"
        result.extend(chunk)
    return bytes(result)


def receive_frame_encrypted(conn, cipher):
    """Read one framed packet from encrypted stream."""
    # Read and decrypt varint length byte by byte
    length = 0
    for i in range(5):
        enc_byte = exact(conn, 1)
        byte = cipher.decrypt(enc_byte)[0]
        length |= (byte & 127) << (i * 7)
        if byte < 128:
            assert 0 < length < 2097156, f"bad frame length {length}"
            # Read and decrypt the frame body
            enc_body = exact(conn, length)
            return cipher.decrypt(enc_body)
    raise AssertionError("bad client frame prefix")


def receive_frame(conn):
    """Read one framed packet."""
    length = 0
    for i in range(5):
        byte = exact(conn, 1)[0]
        length |= (byte & 127) << (i * 7)
        if byte < 128:
            assert 0 < length < 2097156
            return exact(conn, length)
    raise AssertionError("bad client frame prefix")


def take_bytes(body, pos):
    """Extract length-prefixed byte array."""
    length = 0
    for i in range(5):
        byte = body[pos]
        pos += 1
        length |= (byte & 127) << (7 * i)
        if byte < 128:
            assert pos + length <= len(body)
            return body[pos:pos + length], pos + length
    raise AssertionError("bad client array length")


# Independent OpenSSL streaming AES-CFB8 contexts via ctypes
libcrypto = ctypes.CDLL("/opt/homebrew/opt/openssl@3/lib/libcrypto.dylib"
                        if Path("/opt/homebrew/opt/openssl@3/lib/libcrypto.dylib").exists()
                        else ctypes.util.find_library("crypto"))
libcrypto.EVP_aes_128_cfb8.restype = ctypes.c_void_p
libcrypto.EVP_CIPHER_CTX_new.restype = ctypes.c_void_p
libcrypto.EVP_CIPHER_CTX_free.argtypes = [ctypes.c_void_p]
libcrypto.EVP_EncryptInit_ex.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
                                          ctypes.c_char_p, ctypes.c_char_p]
libcrypto.EVP_EncryptInit_ex.restype = ctypes.c_int
libcrypto.EVP_DecryptInit_ex.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
                                          ctypes.c_char_p, ctypes.c_char_p]
libcrypto.EVP_DecryptInit_ex.restype = ctypes.c_int
libcrypto.EVP_EncryptUpdate.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.POINTER(ctypes.c_int),
                                         ctypes.c_char_p, ctypes.c_int]
libcrypto.EVP_EncryptUpdate.restype = ctypes.c_int
libcrypto.EVP_DecryptUpdate.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.POINTER(ctypes.c_int),
                                         ctypes.c_char_p, ctypes.c_int]
libcrypto.EVP_DecryptUpdate.restype = ctypes.c_int
libcrypto.EVP_CIPHER_CTX_set_padding.argtypes = [ctypes.c_void_p, ctypes.c_int]
libcrypto.EVP_CIPHER_CTX_set_padding.restype = ctypes.c_int


class CFB8Session:
    """Persistent AES-CFB8 encryption/decryption contexts."""
    def __init__(self, secret):
        assert len(secret) == 16
        self.encrypt_ctx = libcrypto.EVP_CIPHER_CTX_new()
        self.decrypt_ctx = libcrypto.EVP_CIPHER_CTX_new()
        cipher = libcrypto.EVP_aes_128_cfb8()
        assert libcrypto.EVP_EncryptInit_ex(self.encrypt_ctx, cipher, None, secret, secret) == 1
        assert libcrypto.EVP_DecryptInit_ex(self.decrypt_ctx, cipher, None, secret, secret) == 1
        assert libcrypto.EVP_CIPHER_CTX_set_padding(self.encrypt_ctx, 0) == 1
        assert libcrypto.EVP_CIPHER_CTX_set_padding(self.decrypt_ctx, 0) == 1

    def encrypt(self, data):
        """Encrypt in place."""
        out = ctypes.create_string_buffer(len(data))
        outlen = ctypes.c_int()
        assert libcrypto.EVP_EncryptUpdate(self.encrypt_ctx, out, ctypes.byref(outlen), data, len(data)) == 1
        assert outlen.value == len(data)
        return bytes(out.raw)

    def decrypt(self, data):
        """Decrypt in place."""
        out = ctypes.create_string_buffer(len(data))
        outlen = ctypes.c_int()
        assert libcrypto.EVP_DecryptUpdate(self.decrypt_ctx, out, ctypes.byref(outlen), data, len(data)) == 1
        assert outlen.value == len(data)
        return bytes(out.raw)

    def __del__(self):
        if hasattr(self, 'encrypt_ctx'):
            libcrypto.EVP_CIPHER_CTX_free(self.encrypt_ctx)
        if hasattr(self, 'decrypt_ctx'):
            libcrypto.EVP_CIPHER_CTX_free(self.decrypt_ctx)


def compress_packet(body, threshold):
    """Compress packet body if needed."""
    if threshold < 0:
        return frame(body)
    if len(body) < threshold:
        return frame(b"\x00" + body)
    compressed = zlib.compress(body)
    return frame(vi(len(body)) + compressed)


def decompress_packet(framed, threshold):
    """Check client compression framing independently, then decode its body."""
    if threshold < 0:
        return framed
    if framed[0] == 0:
        assert 0 < len(framed) - 1 < threshold, "client raw packet violates threshold"
        return framed[1:]
    length = 0
    for i in range(5):
        byte = framed[i]
        length |= (byte & 127) << (7 * i)
        if byte < 128:
            assert threshold <= length <= 1024
            inflater = zlib.decompressobj()
            body = inflater.decompress(framed[i + 1:]) + inflater.flush()
            assert inflater.eof and not inflater.unused_data and not inflater.unconsumed_tail
            assert len(body) == length
            return body
    raise AssertionError("bad decompressed length varint")


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
    build = Path(__file__).resolve().parents[1] / "platform/switch/login-probe/tests/build"
    build.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="config-keys-", dir=build) as tmp:
        private = Path(tmp) / "test-1024.pem"
        openssl("genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:1024", "-out", private)
        public_der = openssl("pkey", "-in", private, "-pubout", "-outform", "DER")
        token = b"\x11\x22\x33\x44"

        def fixture(label, threshold_before=256, fragment=False,
                   coalesce=False, large_registry=False, malformed=False, unknown_config=False,
                   code_of_conduct=False, transfer=False, required_pack=False, cancel_after=None):
            """Run one configuration test scenario."""
            stop = threading.Event()
            hello = b"\x01\x00" + string(public_der) + string(token) + b"\x00"

            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                listener.listen(1)
                listener.settimeout(4)
                port = listener.getsockname()[1]
                hello_request = b"\x00" + vi(777) + string(b"127.0.0.1") + port.to_bytes(2, "big") + b"\x02"
                login_request = b"\x00" + string(name) + uuid

                def serve():
                    with listener.accept()[0] as conn:
                        conn.settimeout(5)
                        # Handshake + LoginStart
                        assert receive_frame(conn) == hello_request
                        assert receive_frame(conn) == login_request

                        # Encryption Hello
                        conn.sendall(frame(hello))
                        key_packet = receive_frame(conn)
                        assert key_packet[0] == 1
                        secret_cipher, pos = take_bytes(key_packet, 1)
                        token_cipher, pos = take_bytes(key_packet, pos)
                        assert pos == len(key_packet) and len(secret_cipher) == len(token_cipher) == 128

                        # Decrypt shared secret
                        secret = openssl("pkeyutl", "-decrypt", "-inkey", private,
                                       "-pkeyopt", "rsa_padding_mode:pkcs1", data=secret_cipher)
                        echoed = openssl("pkeyutl", "-decrypt", "-inkey", private,
                                       "-pkeyopt", "rsa_padding_mode:pkcs1", data=token_cipher)
                        assert len(secret) == 16 and echoed == token

                        # Switch to encryption
                        cipher = CFB8Session(secret)

                        # SetCompression before LoginFinished - send as two frames in one encrypted stream
                        comp_frame = frame(b"\x03" + vi(threshold_before)) if threshold_before >= 0 else b""
                        login_success = b"\x02" + uuid + string(name) + b"\x00" + session
                        success_frame = compress_packet(login_success, threshold_before)
                        opening = comp_frame + success_frame
                        if coalesce:
                            opening += compress_packet(b"\x07opaque opening registry", threshold_before)
                        opening = cipher.encrypt(opening)
                        if fragment:
                            for at in range(0, len(opening), 3):
                                conn.sendall(opening[at:at + 3])
                                stop.wait(0.001)
                        else:
                            conn.sendall(opening)

                        # Receive encrypted LoginAcknowledged (id 3)
                        ack_plain = decompress_packet(receive_frame_encrypted(conn, cipher), threshold_before)
                        assert ack_plain == b"\x03", f"bad login ack: {ack_plain.hex()}"

                        # === CONFIGURATION phase ===

                        # Receive brand (id 2 minecraft:brand) and ClientInformation (id 0)
                        brand_plain = decompress_packet(receive_frame_encrypted(conn, cipher), threshold_before)
                        assert brand_plain == b"\x02" + string(b"minecraft:brand") + string(b"CraftRustNX")

                        client_info_plain = decompress_packet(receive_frame_encrypted(conn, cipher), threshold_before)
                        assert client_info_plain == b"\x00\x05en_us\x02\x00\x01\x7f\x01\x00\x01\x00"
                        if cancel_after:
                            assert conn.recv(1) == b"", "client sent data while cancelling configuration"
                            return

                        # Cookie request carries an identifier; response repeats it with absent bytes.
                        cookie_key = b"minecraft:probe_cookie"
                        cookie = compress_packet(b"\x00" + string(cookie_key), threshold_before)
                        conn.sendall(cipher.encrypt(cookie))
                        cookie_reply = decompress_packet(receive_frame_encrypted(conn, cipher), threshold_before)
                        assert cookie_reply == b"\x01" + string(cookie_key) + b"\x00", f"bad cookie reply: {cookie_reply.hex()}"

                        # KnownPacks (id 15) → reply 7 with count 0
                        known = compress_packet(b"\x0f\x01" + string(b"minecraft") + string(b"core") + string(b"26.3"), threshold_before)
                        conn.sendall(cipher.encrypt(known))
                        known_reply = decompress_packet(receive_frame_encrypted(conn, cipher), threshold_before)
                        assert known_reply == b"\x07\x00", f"bad known packs reply: {known_reply.hex()}"

                        # KeepAlive (id 4) with i64 → echo (id 4)
                        keep_id = b"\x12\x34\x56\x78\x9a\xbc\xde\xf0"
                        keep = compress_packet(b"\x04" + keep_id, threshold_before)
                        if fragment:
                            chunks = cipher.encrypt(keep)
                            for byte in chunks:
                                conn.sendall(bytes([byte]))
                                stop.wait(0.001)
                        else:
                            conn.sendall(cipher.encrypt(keep))
                        keep_reply = decompress_packet(receive_frame_encrypted(conn, cipher), threshold_before)
                        assert keep_reply == b"\x04" + keep_id, f"bad keepalive echo"

                        # Ping (id 5) with i32 → pong (id 5)
                        ping_id = b"\x42\x00\x00\x00"
                        ping = compress_packet(b"\x05" + ping_id, threshold_before)
                        conn.sendall(cipher.encrypt(ping))
                        ping_reply = decompress_packet(receive_frame_encrypted(conn, cipher), threshold_before)
                        assert ping_reply == b"\x05" + ping_id, f"bad ping echo"

                        # Large registry (id 7) if requested
                        if large_registry:
                            # Synthetic 70KB registry metadata
                            registry_payload = b"\x00" * 70000
                            registry = compress_packet(b"\x07" + registry_payload, threshold_before)
                            if coalesce:
                                # Send with next packet in one chunk
                                next_pkt = compress_packet(b"\x04" + keep_id, threshold_before)
                                combined = cipher.encrypt(registry + next_pkt)
                                conn.sendall(combined)
                                # Expect echo
                                keep2_plain = decompress_packet(receive_frame_encrypted(conn, cipher), threshold_before)
                                assert keep2_plain == b"\x04" + keep_id
                            else:
                                conn.sendall(cipher.encrypt(registry))

                        # Malformed control packet (bad frame)
                        if malformed:
                            bad = cipher.encrypt(compress_packet(b"\x04\x01", threshold_before))  # Truncated i64 in complete frame
                            conn.sendall(bad)
                            assert conn.recv(1) == b"", "client continued after protocol error"
                            return

                        # Unknown configuration packet
                        if unknown_config:
                            unknown = compress_packet(b"\x7f" + b"unknown", threshold_before)
                            conn.sendall(cipher.encrypt(unknown))
                            assert conn.recv(1) == b"", "client continued after unsupported config"
                            return

                        # Code of conduct (id 20) → requires ACTION_REQUIRED without auto-accept
                        if code_of_conduct:
                            conduct = compress_packet(b"\x14" + string(b"You must agree to play."), threshold_before)
                            conn.sendall(cipher.encrypt(conduct))
                            assert conn.recv(1) == b"", "client continued after code of conduct"
                            return

                        # Transfer (id 12) → unsupported without reconnect
                        if transfer:
                            transfer_pkt = compress_packet(b"\x0c" + string(b"other.server.test") + vi(25565), threshold_before)
                            conn.sendall(cipher.encrypt(transfer_pkt))
                            assert conn.recv(1) == b"", "client continued after transfer"
                            return

                        # Resource pack push (id 9) → declined (id 6), optionally required
                        pack_uuid = uuid_bytes("12345678-1234-5678-1234-567812345678")
                        pack_url = b"https://example.com/pack.zip"
                        pack_hash = b"a" * 40
                        flags = 1 if required_pack else 0
                        pack = compress_packet(b"\x09" + pack_uuid + string(pack_url) + string(pack_hash) +
                                              bytes([flags, 0]), threshold_before)
                        conn.sendall(cipher.encrypt(pack))
                        pack_reply = decompress_packet(receive_frame_encrypted(conn, cipher), threshold_before)
                        assert pack_reply[:17] == b"\x06" + pack_uuid, f"bad pack decline"
                        assert pack_reply[17] == 1, "expected action 1 (declined)"

                        if required_pack:
                            # Server disconnects client after required pack declined
                            reason = b"Required pack declined"
                            disconnect = compress_packet(b"\x02\x08" + len(reason).to_bytes(2, "big") + reason,
                                                         threshold_before)
                            conn.sendall(cipher.encrypt(disconnect))
                            assert conn.recv(1) == b"", "client continued after disconnect"
                            return

                        # Finish (id 3) → ack (id 3)
                        finish = compress_packet(b"\x03", threshold_before)
                        play_body = b"\x32" + b"opaque_play_data" * 100
                        play_pkt = compress_packet(play_body, threshold_before)
                        conn.sendall(cipher.encrypt(finish + play_pkt if coalesce else finish))
                        finish_ack = decompress_packet(receive_frame_encrypted(conn, cipher), threshold_before)
                        assert finish_ack == b"\x03", f"bad finish ack"

                        # === PLAY phase: first packet ===
                        if not coalesce:
                            conn.sendall(cipher.encrypt(play_pkt))

                        # Client reports opaque data not loaded (no world loading)
                        try:
                            assert conn.recv(1) == b"", "client sent play data"
                        except ConnectionResetError:
                            pass

                with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                    server = pool.submit(serve)
                    started = time.monotonic()
                    mode = "normal"
                    if cancel_after:
                        mode = "cancel-configuration"

                    try:
                        result = subprocess.run([runner, "127.0.0.1", str(port), mode],
                                              capture_output=True, text=True, check=False, timeout=18)
                    finally:
                        stop.set()

                    assert result.returncode == 0, (label, result.returncode, result.stdout, result.stderr)
                    try:
                        server.result(timeout=5)
                    except Exception as error:
                        raise AssertionError(f"{label}: client stdout={result.stdout!r} stderr={result.stderr!r}") from error
                    elapsed = time.monotonic() - started

            lines = result.stdout.splitlines()
            values = list(map(int, lines[0].split()))

            # Validate output indices
            # 12:login_accepted 13:config_ack 14:config_frames 15:decoded_bytes
            # 16:registry_frames 17:first_play_id 18:play_body_bytes 19:pack_declined
            # 20:required_pack_declined 21:last_config_id
            # Stages: 14:PLAY_PACKET_SEEN 15:ACTION_REQUIRED 8:UNSUPPORTED 6:DISCONNECTED 9:ERROR 10:CANCELLED
            # Errors: 8:protocol 7:EOF 12:limit

            if malformed:
                assert values[0] == 9 and values[1] == 8, f"{label}: expected protocol error, got {values[:2]}"
            elif unknown_config:
                assert values[0] == 8, f"{label}: expected unsupported, got stage {values[0]}"
            elif code_of_conduct:
                assert values[0] == 15, f"{label}: expected ACTION_REQUIRED, got stage {values[0]}"
            elif transfer:
                assert values[0] == 8, f"{label}: expected unsupported, got stage {values[0]}"
            elif required_pack:
                assert values[0] == 6, f"{label}: expected DISCONNECTED, got stage {values[0]}"
                assert values[12] == 1 and values[13] == 0, f"{label}: expected login accepted before configuration completion"
                assert values[19] == 1 and values[20] == 1, f"{label}: pack/required flags"
            elif cancel_after:
                assert values[0] == 10 and values[12] == 1 and values[13] == 0, f"{label}: expected cancellation during configuration, got {values}"
                assert elapsed < 3
            else:
                assert values[0] == 14, f"{label}: expected PLAY_PACKET_SEEN, got stage {values[0]}"
                assert values[12] == 1 and values[13] == 1, f"{label}: expected login_accepted+config_ack"
                assert values[14] >= 7, f"{label}: expected >=7 config frames, got {values[14]}"
                assert values[17] == 50, f"{label}: expected first_play_id=50, got {values[17]}"
                assert values[18] > 0, f"{label}: expected play body bytes, got {values[18]}"
                if large_registry:
                    assert values[16] >= 1, f"{label}: expected registry frames"
                    assert values[15] > 65536, f"{label}: expected >64KiB decoded"

            assert not result.stderr, (label, result.stderr)
            assert values[9] == 1 and values[10] == 0, (label, values)
            assert values[1] == (8 if malformed else 0), (label, values)
            assert lines[1:4] == [uuid.hex(), session.hex(), name.decode()], (label, lines)
            print(f"PASS {label} ({elapsed:.2f}s)", flush=True)

        # Run scenarios
        fixture("encrypted empty known packs / threshold 256")
        fixture("threshold 0 forces compression", threshold_before=0)
        fixture("fragmented encrypted stream", fragment=True)
        fixture("coalesced >64KiB registry", large_registry=True, coalesce=True)
        fixture("large registry alone", large_registry=True)
        fixture("malformed control packet", malformed=True)
        fixture("unknown configuration packet", unknown_config=True)
        fixture("code of conduct without auto-accept", code_of_conduct=True)
        fixture("transfer without reconnect", transfer=True)
        fixture("required pack declined then disconnect", required_pack=True)
        fixture("encrypted without compression", threshold_before=-1)
        fixture("cancel after config start", cancel_after="live")


if __name__ == "__main__":
    main()
