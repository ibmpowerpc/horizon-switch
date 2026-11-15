# Login Crypto: Minecraft Java 26.3 Encryption

Bounded crypto helpers for offline-profile encryption on LAN servers. Implements RSA PKCS#1 v1.5 and AES-128 CFB8 as required by Minecraft Java Edition protocol.

## Purpose

Negotiates encryption for LAN server connections accepting offline profiles (like TLauncher). The server's `should_authenticate=true` flag does **not** establish server-side enforcement. These helpers perform only the cryptographic negotiation; the server makes its own acceptance/rejection decision. This diagnostic performs no session join or token validation.

## API (include/login_crypto.h)

- `lc_session_create`: Parse DER RSA public key (1024/2048-bit), validate key, encrypt 16-byte shared secret and challenge (1-245 bytes, ≤k-11) with PKCS#1 v1.5 type 2 random padding. Returns ciphertexts (256-byte buffers, actual length = key size).
- `lc_session_encrypt` / `lc_session_decrypt`: AES-128 CFB8 byte streaming with independent shift registers per direction.
- `lc_session_free`: Securely zeroes secret, AES schedules, and shift registers.

## Implementation

### Switch Backend (crypto_switch.c)

- **RSA**: mbedTLS 2.28.10 (`mbedtls_pk_parse_public_key`, `mbedtls_rsa_rsaes_pkcs1_v15_encrypt`)
- **RNG**: libnx CSRNG service (`csrngInitialize`, `csrngGetRandomBytes`, `csrngExit`)
- **AES**: libnx hardware AES (`aes128ContextCreate`, `aes128EncryptBlock`)
- **CFB8**: Portable implementation in `cfb8.c` via block callback

Validates:
- DER TLV structure (no PEM, no trailing garbage)
- RSA key type and size (1024/2048 bits only)
- Challenge length ≤ k-11 (PKCS#1 v1.5 padding constraint)
- CSRNG service Result codes

Lifecycle: Balanced `csrngInitialize`/`csrngExit` per session creation. No global state or frame resets.

### Host Test Backend (tests/crypto_host.c)

- **RSA**: OpenSSL 3 (`d2i_PUBKEY`, `EVP_PKEY_encrypt` with `RSA_PKCS1_PADDING`)
- **RNG**: OpenSSL `RAND_bytes`
- **AES**: OpenSSL EVP AES-128-ECB block calls (padding disabled)
- **CFB8**: Same portable `cfb8.c` via block callback

Same API contract as Switch backend for verification.

### Portable CFB8 (cfb8.c)

NIST SP 800-38A CFB8 mode:
- Encryption: `C[i] = P[i] ⊕ MSB(Block(shift_reg))`, shift ← `LSB(shift_reg) || C[i]`
- Decryption: `P[i] = C[i] ⊕ MSB(Block(shift_reg))`, shift ← `LSB(shift_reg) || C[i]`

Handles arbitrary chunk sizes including zero length. Shift register updated with **ciphertext**, not plaintext (decrypt reads ciphertext from input). Propagates block callback errors cleanly.

### Test Vectors (tests/crypto_vectors.c)

- NIST SP 800-38A AES128-CFB8 encrypt/decrypt vectors (F.3.7, F.3.8)
- Chunk boundary consistency
- Zero-length input
- Independent Tx/Rx shift registers
- Error propagation from block callback

## Build Constraints

### SDK Compile Check (crypto/build/)

```bash
docker run --rm --network none --pull=never \
  -v $PWD:/work -w /work/platform/switch/login-probe/crypto \
  devkitpro/devkita64:20260219 \
  aarch64-none-elf-gcc -c -fPIE -I../include -I/opt/devkitpro/libnx/include \
  -I/opt/devkitpro/portlibs/switch/include -Werror -Wframe-larger-than=32768 \
  -o build/crypto_switch.o crypto_switch.c
```

- Position-independent code (`-fPIE`)
- No automatic buffers >32KB (newlib malloc for the small session context)
- Separate build directory (avoids non-PIE cache bug from `build-login`)
- Compile-only; RSA operations untested until hardware

### Host Tests (tests/crypto_host.c, crypto_vectors.c)

Built against OpenSSL 3 from `/opt/homebrew/opt/openssl@3`:
```bash
gcc -o crypto_vectors tests/crypto_vectors.c crypto/cfb8.c \
  -I/opt/homebrew/opt/openssl@3/include \
  -L/opt/homebrew/opt/openssl@3/lib -lssl -lcrypto
```

Host backend verifies API contract but does **not** prove SDK RNG correctness.

## Limits

- Total experiment size <512 MiB (current: crypto sources + tests <2 MiB)
- Workspace reserve: 10 GiB free (current: 16 GiB available)
- No new dependencies, toolchain installs, Docker images, or network fetches
- SDK image: `devkitpro/devkita64:20260219` (--pull=never)

## Copyright Notices

From SDK headers (for parent project notices):

**mbedTLS 2.28.10** (`/opt/devkitpro/portlibs/switch/include/mbedtls/`):
- Copyright: The Mbed TLS Contributors
- License: Apache-2.0 OR GPL-2.0-or-later

**libnx** (`/opt/devkitpro/libnx/include/switch/`):
- Copyright: libnx Authors
- Services: `csrng.h` by SciresM
- AES: `crypto/aes.h`

## Ownership

Exclusively creates:
- `crypto/{crypto_switch.c, cfb8.c, cfb8.h, README.md, .gitignore}`
- `tests/{crypto_host.c, crypto_vectors.c}`

Parent owns: `include/login_crypto.h` (normative API), build system, UI, transport, other tests.
DeepSeek separately owns Rust codec.
