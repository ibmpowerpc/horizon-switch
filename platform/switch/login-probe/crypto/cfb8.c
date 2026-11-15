/* Portable CFB8 implementation shared between Switch SDK and host tests.
 * CFB8: cipher feedback mode with 8-bit feedback, per NIST SP 800-38A.
 * Encryption: C[i] = P[i] XOR MSB(Block(shift_reg)), shift_reg = LSB(shift_reg) || C[i]
 * Decryption: P[i] = C[i] XOR MSB(Block(shift_reg)), shift_reg = LSB(shift_reg) || C[i]
 * Note: shift register updated with ciphertext C[i], not plaintext P[i].
 * Handles arbitrary chunk sizes including zero length. */
#include "cfb8.h"

int cfb8_process(uint8_t shift_reg[16], cfb8_block_fn block_fn, void *block_ctx,
    uint8_t *data, uint32_t length, int is_decrypt) {
    uint8_t block_out[16];
    int result;
    if (!shift_reg || !block_fn || (!data && length)) return -1;

    for (uint32_t i = 0; i < length; i++) {
        /* Encrypt the current shift register */
        result = block_fn(block_ctx, shift_reg, block_out);
        if (result != 0) {
            volatile uint8_t *wipe = block_out;
            for (int j = 0; j < 16; ++j) wipe[j] = 0;
            return result;
        }

        /* XOR with data byte to get output byte */
        uint8_t input_byte = data[i];
        uint8_t output_byte = input_byte ^ block_out[0];
        data[i] = output_byte;

        /* Update shift register: shift left by 1 byte, append ciphertext byte.
         * For encryption: output_byte is ciphertext.
         * For decryption: input_byte is ciphertext (decrypt reads ciphertext). */
        uint8_t feedback_byte = is_decrypt ? input_byte : output_byte;
        for (int j = 0; j < 15; j++) {
            shift_reg[j] = shift_reg[j + 1];
        }
        shift_reg[15] = feedback_byte;
    }

    volatile uint8_t *wipe = block_out;
    for (int j = 0; j < 16; ++j) wipe[j] = 0;
    return 0;
}
