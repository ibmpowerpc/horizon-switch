/* Minecraft login transport crypto, using the installed Switch SDK.
 * mbedTLS: Copyright The Mbed TLS Contributors, Apache-2.0 OR GPL-2.0-or-later.
 * libnx: Copyright libnx Authors. No session-service authentication is performed.
 */
#include "../include/login_crypto.h"
#include "cfb8.h"
#include <stdlib.h>
#include <string.h>
#include <switch/services/csrng.h>
#include <switch/crypto/aes.h>
#include <mbedtls/pk.h>
#include <mbedtls/rsa.h>
#include <mbedtls/entropy.h>

struct lc_session {
    Aes128Context encrypt_ctx, decrypt_ctx;
    uint8_t encrypt_shift[16], decrypt_shift[16];
};
struct rng_state { Result error; };
static void secure_zero(void *data, size_t length) {
    volatile uint8_t *p = data;
    while (length--) *p++ = 0;
}
static int csrng_random(void *ctx, unsigned char *out, size_t length) {
    struct rng_state *rng = ctx;
    rng->error = csrngGetRandomBytes(out, length);
    return R_FAILED(rng->error) ? MBEDTLS_ERR_ENTROPY_SOURCE_FAILED : 0;
}
static int aes_block(void *ctx, const uint8_t src[16], uint8_t dst[16]) {
    aes128EncryptBlock(ctx, dst, src);
    return 0;
}
/* Reject PEM, indefinite/nonminimal outer DER lengths and trailing bytes. */
static int exact_der(const uint8_t *der, uint32_t length) {
    if (length < 2 || der[0] != 0x30) return 0;
    uint32_t n = der[1], header = 2;
    if (n & 128) {
        const uint32_t bytes = n & 127;
        if (!bytes || bytes > 2 || length < 2 + bytes || !der[2]) return 0;
        n = 0;
        for (uint32_t i = 0; i < bytes; ++i) n = (n << 8) | der[2 + i];
        if (n < 128) return 0;
        header += bytes;
    }
    return header + n == length;
}
int lc_session_create(const uint8_t *der, uint32_t der_len,
    const uint8_t *challenge, uint32_t challenge_len,
    uint8_t encrypted_secret[LC_MAX_KEY_BYTES],
    uint8_t encrypted_challenge[LC_MAX_KEY_BYTES], uint32_t *cipher_len,
    struct lc_session **session, int *detail) {
    if (session) *session = NULL;
    if (cipher_len) *cipher_len = 0;
    if (detail) *detail = 0;
    if (!session || !cipher_len || !detail || !encrypted_secret || !encrypted_challenge)
        return LC_BAD_ARGUMENT;
    memset(encrypted_secret, 0, LC_MAX_KEY_BYTES);
    memset(encrypted_challenge, 0, LC_MAX_KEY_BYTES);
    if (!der || !der_len || der_len > LC_MAX_DER || !challenge ||
        !challenge_len || challenge_len > LC_MAX_CHALLENGE) return LC_BAD_ARGUMENT;
    if (!exact_der(der, der_len)) return LC_KEY;

    mbedtls_pk_context pk;
    mbedtls_pk_init(&pk);
    uint8_t secret[16] = {0};
    struct rng_state rng = {0};
    struct lc_session *created = NULL;
    int result = LC_KEY, error = 0;
    bool rng_open = false;
    error = mbedtls_pk_parse_public_key(&pk, der, der_len);
    if (error) goto done;
    if (mbedtls_pk_get_type(&pk) != MBEDTLS_PK_RSA) goto done;
    const size_t bits = mbedtls_pk_get_bitlen(&pk);
    if (bits != 1024 && bits != 2048) goto done;
    mbedtls_rsa_context *rsa = mbedtls_pk_rsa(pk);
    error = mbedtls_rsa_check_pubkey(rsa);
    if (error) goto done;
    const size_t bytes = mbedtls_rsa_get_len(rsa);
    if (bytes != bits / 8) goto done;
    if (challenge_len > bytes - 11) { result = LC_BAD_ARGUMENT; goto done; }

    result = LC_RANDOM;
    rng.error = csrngInitialize();
    if (R_FAILED(rng.error)) goto done;
    rng_open = true;
    rng.error = csrngGetRandomBytes(secret, sizeof(secret));
    if (R_FAILED(rng.error)) goto done;
    result = LC_ENCRYPT;
    error = mbedtls_rsa_rsaes_pkcs1_v15_encrypt(rsa, csrng_random, &rng,
        MBEDTLS_RSA_PUBLIC, sizeof(secret), secret, encrypted_secret);
    if (error) goto done;
    error = mbedtls_rsa_rsaes_pkcs1_v15_encrypt(rsa, csrng_random, &rng,
        MBEDTLS_RSA_PUBLIC, challenge_len, challenge, encrypted_challenge);
    if (error) goto done;

    created = calloc(1, sizeof(*created));
    if (!created) { result = LC_MEMORY; goto done; }
    aes128ContextCreate(&created->encrypt_ctx, secret, true);
    aes128ContextCreate(&created->decrypt_ctx, secret, true);
    memcpy(created->encrypt_shift, secret, 16);
    memcpy(created->decrypt_shift, secret, 16);
    *session = created;
    *cipher_len = (uint32_t)bytes;
    result = LC_OK;
done:
    secure_zero(secret, sizeof(secret));
    mbedtls_pk_free(&pk);
    if (rng_open) csrngExit();
    if (result != LC_OK) {
        if (R_FAILED(rng.error)) { result = LC_RANDOM; *detail = (int)rng.error; }
        else *detail = error;
        secure_zero(encrypted_secret, LC_MAX_KEY_BYTES);
        secure_zero(encrypted_challenge, LC_MAX_KEY_BYTES);
    }
    return result;
}
int lc_session_encrypt(struct lc_session *session, uint8_t *data, uint32_t length) {
    if (!session || (!data && length)) return LC_BAD_ARGUMENT;
    return cfb8_process(session->encrypt_shift, aes_block, &session->encrypt_ctx, data, length, 0);
}
int lc_session_decrypt(struct lc_session *session, uint8_t *data, uint32_t length) {
    if (!session || (!data && length)) return LC_BAD_ARGUMENT;
    return cfb8_process(session->decrypt_shift, aes_block, &session->decrypt_ctx, data, length, 1);
}
void lc_session_free(struct lc_session *session) {
    if (session) { secure_zero(session, sizeof(*session)); free(session); }
}
