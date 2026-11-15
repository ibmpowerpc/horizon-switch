# Encrypted login transition: exact source audit

Companion to [switch-login-audit.md](switch-login-audit.md), which covers the probe's framing,
limits and unsupported paths. This one reads the Horizon sources that define the encrypted
transition and prices a reusable `no_std` crypto path. Sources are `git show HEAD:` blobs of fork
commit `cab6fc025`; `crates/` is not in this working tree and `.local/` is absent, so no vanilla
decompiled oracle was consulted.

## Where the client branch is

`crates/horizon-client/src/net.rs`, `fn run` (blob line 756), `login::Clientbound::Hello` arm
(852–899). In order: decode the packet, build `RsaPublicKey::from_der` (858), draw a 16-byte
secret (859), optionally do the session join, then:

```rust
c.send(&login::Serverbound::Key(login::Key {
    key: key.encrypt(&secret)?,
    challenge: key.encrypt(&hello.challenge)?,
}).encode())?;
c.enable_encryption(&secret);
```

Both facts that matter sit outside the `if hello.should_authenticate` block (857–886):

- **The flag gates only the session join.** The RSA key parse, the secret draw, the
  `EncryptionResponse` and the cipher switch happen for `false` as well. `false` therefore means
  "no Mojang `joinServer`", not "no encryption".
- **With no account the `true` path ends.** `auth.ok_or_else(|| keyed("horizon.multiplayer.not_signed_in"))`
  is only forgiven by `continues_after_failed_join(lan, signed_in)` (1486–1496), which is
  `lan && signed_in`. An unsigned client gets `AUTH_OR_ENCRYPTION_REQUIRED`-equivalent refusal on
  `true`, and nothing else. The probe currently has no account session with which to complete that path.
- The file's own server half (2152–2170) mirrors the exchange: it sends
  `should_authenticate: true`, waits for one key packet, then `enable_encryption`.

## Wire schema (protocol 777)

`crates/horizon-protocol/src/packets/login.rs`:

- Clientbound `Hello` = `server_id: StrN<20>`, `public_key: Bytes`, `challenge: Bytes`,
  `should_authenticate: Bool` (26–31). `server_id` is empty in current vanilla practice; it is
  still needed for `server_hash` when the flag is `true`.
- Serverbound `Key` = `key: Bytes`, `challenge: Bytes` (52). `key` is the RSA/PKCS#1-v1.5
  ciphertext of the 16-byte secret, `challenge` the ciphertext of the server's challenge bytes
  exactly as received; each ciphertext is `size()` bytes (128 for the usual 1024-bit modulus).
- `RsaPublicKey::encrypt` (`rsa.rs` 348) rejects `message.len() + 11 > k`, fills PKCS#1 v1.5
  type-2 padding with *non-zero random* bytes, and exponentiates `e = 65537`. Key parsing is
  X.509 `SubjectPublicKeyInfo` only (277), modulus up to 16384 bits (260–265).
- `LoginFinished` is decoded as `GameProfile` **plus a 16-byte session UUID** (32). That tail is a
  Horizon reference assumption; the probe mirrors it and no hardware run has reached it.

SHA-1 (`crypto.rs::server_hash`) is needed only for the `true` path; MD5 (offline UUID) already
exists in the probe. So a `false` path needs no hashing at all.

## Where the cipher starts, and the residual-byte trap

`crates/horizon-protocol/src/connection.rs`:

- `enable_encryption` (441–448) builds `Cfb8::new(secret, secret)` for each direction — key **and**
  IV are the same 16 bytes — and then decrypts whatever is already buffered:
  `decrypt.decrypt(&mut r.buffer[r.start..r.end])`.
- The `Key` frame is written before that call, so it is the **last plaintext frame**; everything
  after, including the next `SetCompression` and every later login/configuration/play frame, is
  encrypted.
- `frame_into` (706–725) compresses first (`frame::contents(body, threshold)`) and then encrypts
  the whole framed bytes — varint length included. Encryption is outermost. Reading is the mirror
  (`fill_from`, 621–648).
- Coalesced reads: bytes the server sent after processing the key may already sit in the read
  buffer when the secret becomes known. They start at CFB8 stream position 0 (the server's stream),
  so they must be decrypted in place exactly once, before any varint is parsed, and the register
  must continue from them. A C port that instead drops the buffer, or decrypts it twice, desyncs
  the register and every later byte.
- Each byte's keystream is `AES(register)`; encryption updates the register with the ciphertext
  byte, decryption with the received byte. A single direction mistake only shows up as garbage
  framing much later.

## Can `should_authenticate = false` legitimately need RSA and AES?

By the sources above, yes on the client side: Horizon's client encrypts unconditionally. Whether a
given vanilla 26.3 world sends `Hello` at all when `online-mode=false` is the complement question,
and it is not answerable from this checkout: the claim that a socket (non-memory) login is still
encrypted offline is Horizon-derived, not oracle-verified here. This is exactly why the probe must
record the flag rather than infer the world's mode: `true` requests a session join, `false`
with an encryption request implies transport encryption without a session, and neither is a bypass.

## Cheapest reusable `no_std` crypto path (cache facts, nothing built)

- **AES**: `aes 0.8.4` is already in `~/.cargo/registry` (src and `.crate`, MIT OR Apache-2.0),
  and already in Horizon's `Cargo.lock`. It has no `build.rs`; `aes_armv8` is only a manual `--cfg`,
  so on a Switch-class `aarch64` target the ARMv8 path is off. Its only target-gated dependency is
  `cpufeatures 0.2.17` (also cached); on targets that are not Apple/Linux/Android
  `__detect_target_features!` expands to `false` (`cpufeatures/src/aarch64.rs:160`), so the soft
  AES is selected at compile time with no libc/auxv. `cipher 0.4.4`, `generic-array 0.14.7`,
  `typenum 1.20.0`, `cfg-if 1.0.4/1.0.5` are cached too. Nothing needs fetching.
- **CFB8**: `crypto.rs` 10–52. The only `std` items on the current path are
  `std::array::from_fn` (42) and, in the x86 backend, `std::slice` (`crypto/blocks.rs` 116). With
  the soft `aes` backend and `core::array::from_fn` the module is `alloc`-free. Not edited here.
- **RSA**: no `rsa` crate is cached (only `num-bigint 0.4.8`, `num-integer`, `num-traits`; all
  `no_std + alloc` capable, but `modpow` over `BigUint` still needs allocation and a serializer for
  the SPKI). Horizon's own `rsa.rs` is MIT (workspace `license = "MIT"`, `Cargo.toml:62`) and
  self-contained: lines 1–370 cover `Big`, `Mont` (Montgomery `pow`), `der`, `from_der`, `size`,
  `apply`, `encrypt`. Extraction cost is ~350 lines, `Vec`-backed, needing only `Error(String)`
  and `random_bytes`; the signature/verify half (400+) and the private-key half (537+) are not
  needed.
- **Randomness is the one blocking dependency.** `crypto::random_bytes` is `getrandom 0.3`, which
  has no Switch backend; the secret *and* the PKCS#1 padding need platform randomness. That is the
  SDK boundary and is deliberately not audited here.

## Minimum honest probe extension

`platform/switch/login-probe/codec/src/decode.rs:199–205` already parses the boolean, but
`codec/src/ffi.rs:82` matches `Event::Hello { .. }` and drops it, and `struct lp_event`
(`include/login_codec.h:10`) has no field for it. The smallest truthful change is therefore not
crypto: add a `uint8_t should_authenticate` to `lp_event`, set it in `event_of`, and print
`AUTH_OR_ENCRYPTION_REQUIRED (should_authenticate=…)` next to the stage in
`probe/source-login/main.c`. The stage keeps its name, the worker keeps closing after the event,
and the UI must not relabel the world as offline without that value.

Only after AES + RSA + a platform RNG exist can the `false` branch be attempted: encrypt the
secret and challenge, send the key frame, switch both directions to CFB8, keep the existing
threshold/compression handling, decrypt residual bytes, and read on to `LoginFinished`. The `true`
branch stays a clear auth-required result; it cannot be completed without a Microsoft session.

## Unresolved

- The flag value sent by the tested LAN world; its server configuration is not established.
- Real `LoginFinished` layout: the 16-byte session tail is unverified against a real 26.3 server.
- Challenge length (vanilla-sized, trust the packet), typical 1024-bit DER key, empty `server_id`.
- Whether the Switch target's `aes` build stays soft and `cpufeatures` stays libc-free: reasoned
  from source, not compiled — no new targets or builds were run.
- Heap: the probe is currently allocation-free; RSA introduces `alloc`, so the Switch heap budget
  needs a decision when the false path is implemented.
