/* NIST SP 800-38A F.3.7/F.3.8 AES128 CFB8; real OpenSSL ECB blocks,
 * shared CFB8 implementation, independent expected ciphertext. */
#include "../crypto/cfb8.h"
#include <openssl/evp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static void require(int ok, const char *name) {
    if (!ok) { fprintf(stderr, "FAIL %s\n", name); exit(1); }
}
static int aes_block(void *ctx, const uint8_t src[16], uint8_t dst[16]) {
    int written = 0;
    return EVP_EncryptUpdate(ctx, dst, &written, src, 16) == 1 && written == 16 ? 0 : -1;
}
static int fail_block(void *ctx, const uint8_t src[16], uint8_t dst[16]) {
    (void)ctx; (void)src; (void)dst;
    return -999;
}
int main(void) {
    const uint8_t key[16] = {0x2b,0x7e,0x15,0x16,0x28,0xae,0xd2,0xa6,0xab,0xf7,0x15,0x88,0x09,0xcf,0x4f,0x3c};
    const uint8_t iv[16] = {0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15};
    const uint8_t plain[16] = {0x6b,0xc1,0xbe,0xe2,0x2e,0x40,0x9f,0x96,0xe9,0x3d,0x7e,0x11,0x73,0x93,0x17,0x2a};
    const uint8_t expected[16] = {0x3b,0x79,0x42,0x4c,0x9c,0x0d,0xd4,0x36,0xba,0xce,0x9e,0x0e,0xd4,0x58,0x6a,0x4f};
    EVP_CIPHER_CTX *aes = EVP_CIPHER_CTX_new();
    require(aes != NULL && EVP_EncryptInit_ex(aes, EVP_aes_128_ecb(), NULL, key, NULL) == 1 &&
            EVP_CIPHER_CTX_set_padding(aes, 0) == 1, "AES context");
    uint8_t shift[16], data[16];
    memcpy(shift, iv, 16); memcpy(data, plain, 16);
    require(cfb8_process(shift, aes_block, aes, data, 16, 0) == 0 &&
            !memcmp(data, expected, 16), "NIST encrypt");
    memcpy(shift, iv, 16); memcpy(data, expected, 16);
    require(cfb8_process(shift, aes_block, aes, data, 16, 1) == 0 &&
            !memcmp(data, plain, 16), "NIST decrypt");
    memcpy(shift, iv, 16); memcpy(data, plain, 16);
    require(cfb8_process(shift, aes_block, aes, data, 3, 0) == 0 &&
            cfb8_process(shift, aes_block, aes, data + 3, 9, 0) == 0 &&
            cfb8_process(shift, aes_block, aes, data + 12, 4, 0) == 0 &&
            !memcmp(data, expected, 16), "chunk boundaries");
    memcpy(shift, iv, 16);
    require(cfb8_process(shift, aes_block, aes, NULL, 0, 0) == 0 &&
            !memcmp(shift, iv, 16), "zero length");
    uint8_t tx[16], rx[16];
    memcpy(tx, iv, 16); memcpy(rx, iv, 16); memcpy(data, plain, 16);
    require(cfb8_process(tx, aes_block, aes, data, 16, 0) == 0 &&
            !memcmp(rx, iv, 16) && cfb8_process(rx, aes_block, aes, data, 16, 1) == 0 &&
            !memcmp(data, plain, 16), "independent directions");
    memcpy(shift, iv, 16); memcpy(data, plain, 16);
    require(cfb8_process(shift, fail_block, NULL, data, 16, 0) == -999 &&
            !memcmp(shift, iv, 16) && !memcmp(data, plain, 16), "block error propagation");
    EVP_CIPHER_CTX_free(aes);
    puts("PASS 6 CFB8 scenarios");
    return 0;
}
