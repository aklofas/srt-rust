#ifndef TST_EMB_DIAG_H
#define TST_EMB_DIAG_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Direct ARM semihosting SYS_WRITE0 — safe from fault/assert context
 * (no newlib, no locks, no heap). */
void tst_diag_write0(const char *s);
/* Format v as 8 lowercase hex digits into a caller buffer (>= 9 bytes). */
void tst_diag_hex32(uint32_t v, char out[9]);
/* Format v as decimal into a caller buffer (>= 11 bytes). */
void tst_diag_u32(uint32_t v, char out[11]);
/* Print "FAIL[<label>]\n" and semihosting-_exit(1). Never returns. */
__attribute__((noreturn)) void tst_diag_fail(const char *label);
/* configASSERT hook: prints FAIL[assert] file:line, exits 1. */
__attribute__((noreturn)) void vAssertCalled(const char *file, unsigned long line);
#ifdef __cplusplus
}
#endif
#endif
