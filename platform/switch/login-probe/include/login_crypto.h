#ifndef CRAFTRUSTNX_LOGIN_CRYPTO_H
#define CRAFTRUSTNX_LOGIN_CRYPTO_H
#include <stdint.h>

enum { LC_OK = 0, LC_BAD_ARGUMENT = -1, LC_MEMORY = -2, LC_RANDOM = -3,
    LC_KEY = -4, LC_ENCRYPT = -5, LC_MAX_KEY_BYTES = 256,
    LC_MAX_DER = 1024, LC_MAX_CHALLENGE = 245 };
struct lc_session;
/* Caller supplies valid non-overlapping input regions and 256-byte writable
 * ciphertext buffers. Length/out session reset on failure. Supported RSA keys:
 * 1024/2048 bits, DER public key. PKCS#1 v1.5 random padding, no fixed seeds.
 * Shared secret stays in session; never returned/logged. detail is SDK error.
 * All buffers/outputs must remain valid for the full call. */
int lc_session_create(const uint8_t *der, uint32_t der_len,
    const uint8_t *challenge, uint32_t challenge_len,
    uint8_t encrypted_secret[LC_MAX_KEY_BYTES],
    uint8_t encrypted_challenge[LC_MAX_KEY_BYTES], uint32_t *cipher_len,
    struct lc_session **session, int *detail);
/* Independent AES-128 CFB8 directions; shared secret is both initial IV and key.
 * In-place byte streaming, no frame resets. Valid caller-owned data region. */
int lc_session_encrypt(struct lc_session *session, uint8_t *data, uint32_t length);
int lc_session_decrypt(struct lc_session *session, uint8_t *data, uint32_t length);
void lc_session_free(struct lc_session *session);
#endif
