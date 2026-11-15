/* Real OpenSSL host backend for socket fixtures. Switch RSA/RNG remain
 * hardware-unverified; both backends share the exact CFB8 byte algorithm. */
#include "../include/login_crypto.h"
#include "../crypto/cfb8.h"
#include <stdlib.h>
#include <string.h>
#include <openssl/evp.h>
#include <openssl/rsa.h>
#include <openssl/rand.h>
#include <openssl/err.h>
#include <openssl/x509.h>
struct lc_session {
    EVP_CIPHER_CTX *encrypt_ctx, *decrypt_ctx;
    uint8_t encrypt_shift[16], decrypt_shift[16];
};
static int aes_block(void *ctx, const uint8_t src[16], uint8_t dst[16]) {
    int written = 0;
    if (EVP_EncryptUpdate(ctx, dst, &written, src, 16) != 1 || written != 16)
        return LC_ENCRYPT;
    return LC_OK;
}
void lc_session_free(struct lc_session *session) {
    if (session) {
        EVP_CIPHER_CTX_free(session->encrypt_ctx);
        EVP_CIPHER_CTX_free(session->decrypt_ctx);
        OPENSSL_cleanse(session, sizeof(*session));
        free(session);
    }
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
    if (!der || !der_len || der_len > LC_MAX_DER || der[0] != 0x30 ||
        !challenge || !challenge_len || challenge_len > LC_MAX_CHALLENGE) return LC_BAD_ARGUMENT;

    int result = LC_KEY;
    const unsigned char *cursor = der;
    EVP_PKEY *pkey = d2i_PUBKEY(NULL, &cursor, der_len);
    EVP_PKEY_CTX *ctx = NULL;
    struct lc_session *created = NULL;
    uint8_t secret[16] = {0};
    size_t bytes = 0, written = 0;
    if (!pkey || cursor != der + der_len || EVP_PKEY_id(pkey) != EVP_PKEY_RSA) goto done;
    const int bits = EVP_PKEY_bits(pkey);
    if (bits != 1024 && bits != 2048) goto done;
    bytes = (size_t)EVP_PKEY_size(pkey);
    if (bytes != (size_t)bits / 8) goto done;
    if (challenge_len > bytes - 11) { result = LC_BAD_ARGUMENT; goto done; }
    ctx = EVP_PKEY_CTX_new(pkey, NULL);
    if (!ctx) { result = LC_MEMORY; goto done; }
    if (EVP_PKEY_public_check(ctx) != 1) goto done;
    result = LC_RANDOM;
    if (RAND_bytes(secret, 16) != 1) goto done;
    result = LC_ENCRYPT;
    if (EVP_PKEY_encrypt_init(ctx) <= 0 ||
        EVP_PKEY_CTX_set_rsa_padding(ctx, RSA_PKCS1_PADDING) <= 0) goto done;
    written = bytes;
    if (EVP_PKEY_encrypt(ctx, encrypted_secret, &written, secret, 16) <= 0 || written != bytes) goto done;
    written = bytes;
    if (EVP_PKEY_encrypt(ctx, encrypted_challenge, &written, challenge, challenge_len) <= 0 || written != bytes) goto done;

    created = calloc(1, sizeof(*created));
    if (!created) { result = LC_MEMORY; goto done; }
    created->encrypt_ctx = EVP_CIPHER_CTX_new();
    created->decrypt_ctx = EVP_CIPHER_CTX_new();
    if (!created->encrypt_ctx || !created->decrypt_ctx) { result = LC_MEMORY; goto done; }
    if (EVP_EncryptInit_ex(created->encrypt_ctx, EVP_aes_128_ecb(), NULL, secret, NULL) != 1 ||
        EVP_EncryptInit_ex(created->decrypt_ctx, EVP_aes_128_ecb(), NULL, secret, NULL) != 1 ||
        EVP_CIPHER_CTX_set_padding(created->encrypt_ctx, 0) != 1 ||
        EVP_CIPHER_CTX_set_padding(created->decrypt_ctx, 0) != 1) goto done;
    memcpy(created->encrypt_shift, secret, 16);
    memcpy(created->decrypt_shift, secret, 16);
    *session = created;
    *cipher_len = (uint32_t)bytes;
    created = NULL;
    result = LC_OK;
done:
    OPENSSL_cleanse(secret, sizeof(secret));
    lc_session_free(created);
    EVP_PKEY_CTX_free(ctx);
    EVP_PKEY_free(pkey);
    if (result != LC_OK) {
        *detail = (int)ERR_get_error();
        OPENSSL_cleanse(encrypted_secret, LC_MAX_KEY_BYTES);
        OPENSSL_cleanse(encrypted_challenge, LC_MAX_KEY_BYTES);
    }
    return result;
}
int lc_session_encrypt(struct lc_session *session, uint8_t *data, uint32_t length) {
    if (!session || (!data && length)) return LC_BAD_ARGUMENT;
    return cfb8_process(session->encrypt_shift, aes_block, session->encrypt_ctx, data, length, 0);
}
int lc_session_decrypt(struct lc_session *session, uint8_t *data, uint32_t length) {
    if (!session || (!data && length)) return LC_BAD_ARGUMENT;
    return cfb8_process(session->decrypt_shift, aes_block, session->decrypt_ctx, data, length, 1);
}
