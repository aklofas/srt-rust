#include "diag.h"

void tst_diag_write0(const char *s) {
    register uint32_t r0 __asm("r0") = 0x04u; /* SYS_WRITE0 */
    register const char *r1 __asm("r1") = s;
    /* "+r" (in-out) + "cc" clobber: the ARM semihosting spec says r0 holds
     * the return value after the BKPT, so both registers are corrupted by
     * the host.  Without in-out constraints the compiler may reuse r0/r1
     * for live values after this call at -O2. */
    __asm volatile("bkpt 0xab" : "+r"(r0), "+r"(r1) : : "memory", "cc");
}

void tst_diag_hex32(uint32_t v, char out[9]) {
    static const char d[] = "0123456789abcdef";
    for (int i = 0; i < 8; i++) out[i] = d[(v >> (28 - 4 * i)) & 0xF];
    out[8] = '\0';
}

void tst_diag_u32(uint32_t v, char out[11]) {
    char tmp[11]; int n = 0;
    do { tmp[n++] = (char)('0' + v % 10); v /= 10; } while (v);
    for (int i = 0; i < n; i++) out[i] = tmp[n - 1 - i];
    out[n] = '\0';
}

extern void _exit(int) __attribute__((noreturn));

void tst_diag_fail(const char *label) {
    tst_diag_write0("FAIL[");
    tst_diag_write0(label);
    tst_diag_write0("]\n");
    _exit(1);
}

void vAssertCalled(const char *file, unsigned long line) {
    __asm volatile("cpsid i"); /* no further preemption on the fatal path */
    char num[11];
    tst_diag_write0("FAIL[assert] ");
    tst_diag_write0(file);
    tst_diag_write0(":");
    tst_diag_u32((uint32_t)line, num);
    tst_diag_write0(num);
    tst_diag_write0("\n");
    _exit(1);
}
