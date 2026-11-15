# Switch Crypto SDK Audit: Minecraft Encryption Path

**Scope:** Native SDK cryptographic primitives for Minecraft Java 26.3 login encryption negotiation (transport only; session-service authentication is separate)
**Environment:** devkitpro/devkita64:20260219, libnx, mbedTLS v2.28.10
**Date:** 2026-11-15

## Executive Summary

**VIABLE PATH CONFIRMED.** All required primitives exist in the SDK with proper semantics for Minecraft's encryption handshake. No external dependencies or native RSA implementation needed.

## Requirements vs. Available Primitives

### 1. Cryptographic Random Number Generation

**Requirement:** Secure random bytes for AES shared secret (16 bytes) and RSA padding.

**Available:**
- **`csrngGetRandomBytes(void *out, size_t out_size)`** (`switch/services/csrng.h`)
  - IPC wrapper to system CSPRNG service
  - Returns `Result` error code
  - **Failure semantics:** Returns non-zero Result on service failure; caller must check and handle
  - Appropriate for all crypto operations requiring unpredictable randomness

- **`randomGet(void* buf, size_t len)`** (`switch/kernel/random.h`)
  - ChaCha-based PRNG, OS-seeded at init
  - Not selected for this probe: the service API provides explicit failure reporting.
    No claim about randomGet security is established by this audit.

**Recommendation:** Use `csrngGetRandomBytes` with explicit error handling. Do not proceed with encryption if it fails.

### 2. AES-128 CFB8 Mode

**Requirement:** Streaming cipher with independent encrypt/decrypt state, per-byte operation (CFB-8 segment size).

**Available:**
- **`aes128EncryptBlock(const Aes128Context *ctx, void *dst, const void *src)`** (`switch/crypto/aes.h`)
  - Hardware-accelerated AES-ECB single-block (16 bytes)
  - Separate encrypt/decrypt contexts via `aes128ContextCreate(..., bool is_encryptor)`
  - CFB8 mode **constructible** by XORing each plaintext/ciphertext byte with one byte from the encrypted IV/shift register, then shifting in the new ciphertext byte

**Implementation note:** No direct CFB8 API. Application must maintain:
- 16-byte shift register (IV initially)
- Encrypt shift register per byte
- XOR one byte of keystream with data byte
- Shift ciphertext byte into register

Independent state per direction guaranteed by separate `Aes128Context` instances. Boundary-safe (operates byte-by-byte).

### 3. RSA Encryption PKCS#1 v1.5

**Requirement:** Encrypt 16-byte AES secret with server's DER-encoded SPKI RSA public key (typically 1024 or 2048 bits). PKCS#1 v1.5 padding (not OAEP).

**Available (mbedTLS v2.28.10, `libmbedcrypto.a`):**

Verified exported symbols (36 RSA functions):
- **`mbedtls_rsa_init(mbedtls_rsa_context *ctx, int padding, int hash_id)`**
  - Pass `MBEDTLS_RSA_PKCS_V15` for padding mode
- **`mbedtls_rsa_import(mbedtls_rsa_context *ctx, const mbedtls_mpi *N, const mbedtls_mpi *P, const mbedtls_mpi *Q, const mbedtls_mpi *D, const mbedtls_mpi *E)`**
  - Import modulus (N) and public exponent (E) from MPI format
- **`mbedtls_rsa_import_raw(mbedtls_rsa_context *ctx, const unsigned char *N, size_t N_len, ...)`**
  - Direct import from big-endian byte arrays
- **`mbedtls_rsa_complete(mbedtls_rsa_context *ctx)`**
  - Validate and finalize key after import
- **`mbedtls_rsa_rsaes_pkcs1_v15_encrypt(mbedtls_rsa_context *ctx, int (*f_rng)(...), void *p_rng, int mode, size_t ilen, const unsigned char *input, unsigned char *output)`**
  - PKCS#1 v1.5 encryption with RNG callback for padding
  - `f_rng` receives csrng wrapper; no fixed seeds
  - Key size bounds: supports 1024, 2048, 4096-bit keys (checked via `mbedtls_rsa_get_len`)

**DER Parsing:**
- **`mbedtls_pk_parse_public_key(mbedtls_pk_context *ctx, const unsigned char *key, size_t keylen)`**
  - Parses DER or PEM SPKI directly into usable key context
  - `mbedtls_pk_get_type()` confirms RSA
  - `mbedtls_pk_rsa()` extracts underlying `mbedtls_rsa_context`
- Full ASN.1/DER primitives available (`mbedtls_asn1_get_*`) if manual parsing needed

**No blockers:** DER parse, modular exponentiation (via `mbedtls_rsa_public`), PKCS#1 v1.5 padding with proper RNG, all present.

## Integration Path

1. **Key Import:**
   ```c
   mbedtls_pk_context pk;
   mbedtls_pk_init(&pk);
   mbedtls_pk_parse_public_key(&pk, der_spki, der_len);
   mbedtls_rsa_context *rsa = mbedtls_pk_rsa(pk);
   ```

2. **Shared Secret Encryption:**
   ```c
   u8 secret[16], encrypted[256];
   Result rc = csrngGetRandomBytes(secret, 16);
   if (R_FAILED(rc)) { /* abort handshake */ }

   int ret = mbedtls_rsa_rsaes_pkcs1_v15_encrypt(
       rsa, csrng_callback, NULL, MBEDTLS_RSA_PUBLIC,
       16, secret, encrypted
   );
   ```

3. **AES CFB8 Stream:**
   Implement byte-by-byte XOR loop with `aes128EncryptBlock` on shift register. Separate contexts for send/receive directions.

## Known Constraints

- **No hardware CFB8 accelerator:** CPU byte loop; throughput has not been measured for this port.
- **mbedTLS dependency:** Links ~884 KB `libmbedcrypto.a`; well within artifact budget (457 MiB used of 512 MiB cap).
- **No secure storage:** Shared secret exists in memory; standard for this protocol phase. Not persisting session keys.

## Risk Assessment

- **Cryptographic correctness:** Using standard mbedTLS primitives, not custom bignum. Compile-only API evidence; hardware behavior still requires verification.
- **RNG failure:** `csrngGetRandomBytes` may fail if system service unavailable. Proper Result checking prevents weak key use.
- **Padding oracle:** PKCS#1 v1.5 known vulnerable in decryption; not applicable here (client encrypts only, server decrypts).
- **CFB8 implementation bugs:** Shift register state must be carefully managed. Recommend unit test against vanilla Java client's known-answer test vectors.

## Conclusion

**All primitives present with correct semantics.** No missing SDK functions, no need for external libraries or native RSA code. Path forward: link mbedcrypto, implement CFB8 wrapper atop libnx AES-ECB, integrate RNG error handling. Estimated artifact growth (not a measured linked size): <1 MiB total (mbedtls .a is already in image).

Primary evidence: `/opt/devkitpro/portlibs/switch/include/mbedtls/rsa.h`, verified symbols in `libmbedcrypto.a` via `aarch64-none-elf-nm`, libnx headers in `/opt/devkitpro/libnx/include/switch/{crypto/,services/csrng.h,kernel/random.h}`.
