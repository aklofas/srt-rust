/* Minimal Cortex-M4F startup for QEMU mps2-an386 + newlib/rdimon semihosting.
 *
 * Root-cause note (discovered during toolchain de-risk):
 *   __libc_init_array() calls _init() first.  _init() is a stub assembled by
 *   the linker from crti.o (prologue) + crtn.o (epilogue); without explicit
 *   .init/.fini sections in the linker script the epilogue pop/bx-lr lands in
 *   the wrong place and _init() never returns, hanging __libc_init_array().
 *   Fix: the linker script sequences KEEP(*crti.o(.init)) … KEEP(*crtn.o(.init)).
 *
 *   Additionally, newlib's _exit() → _kill_shared() probes the host debugger
 *   via _has_ext_exit_extended(); QEMU 8.2 handles this but can be fragile.
 *   Overriding _exit() with a direct SYS_EXIT semihosting BKPT guarantees a
 *   clean exit regardless of the newlib version's probing strategy.
 */
#include <stdint.h>
#include <errno.h>

extern uint32_t _sidata, _sdata, _edata, _sbss, _ebss, _estack;
extern char end, _heap_end;
extern int  main(void);
extern void __libc_init_array(void);
extern void initialise_monitor_handles(void); /* rdimon */

/* Forward declaration — defined at the bottom of this file. */
__attribute__((noreturn)) void _exit(int status);

void Reset_Handler(void);
static void Default_Handler(void) { for (;;) {} }

__attribute__((section(".isr_vector"), used))
void (* const g_vectors[])(void) = {
    (void (*)(void))(&_estack), /* [0] initial MSP */
    Reset_Handler,              /* [1] reset       */
    Default_Handler,            /* [2] NMI         */
    Default_Handler,            /* [3] HardFault   */
};

void Reset_Handler(void) {
    /* Enable the FPU (CP10/CP11 full access) — hard-float target. */
    volatile uint32_t *cpacr = (volatile uint32_t *)0xE000ED88u;
    *cpacr |= (0xFu << 20);
    __asm volatile("dsb"); __asm volatile("isb");

    /* Copy .data from FLASH to RAM. */
    for (uint32_t *s = &_sidata, *d = &_sdata; d < &_edata; ) *d++ = *s++;
    /* Zero .bss. */
    for (uint32_t *d = &_sbss; d < &_ebss; ) *d++ = 0;

    initialise_monitor_handles();
    __libc_init_array();
    _exit(main());
    for (;;) {} /* unreachable; satisfies noreturn analysis */
}

/* Self-contained heap for newlib malloc.  librdimon.a defines _sbrk as a
   weak (W) symbol; this strong (T) definition takes precedence so we control
   heap limits explicitly.  If the linker reports a duplicate definition, drop
   this function and rely on librdimon's version. */
void *_sbrk(int incr) {
    static char *brk = 0;
    if (!brk) brk = &end;
    /* Bound BOTH ends: a negative incr must not underflow below the heap base
       (&end) into .bss/.data. Set errno=ENOMEM on failure per the newlib
       contract so malloc can distinguish OOM. */
    if (brk + incr > &_heap_end || brk + incr < &end) {
        errno = ENOMEM;
        return (void *)-1;
    }
    char *prev = brk; brk += incr; return prev;
}

/* Override newlib's _exit() with a direct ARM semihosting SYS_EXIT BKPT.
 * Newlib's default path (_exit → _kill_shared → _has_ext_exit_extended) probes
 * the debugger to detect the "extended exit" extension; this probing is fragile
 * on some QEMU versions.  The raw SYS_EXIT (0x18) call below is unambiguous:
 *   R0 = 0x18 (SYS_EXIT)
 *   R1 = reason code (ADP_Stopped_ApplicationExit=0x20026 for exit(0),
 *                     ADP_Stopped_RunTimeError=0x20023 for any non-zero status)
 * QEMU maps ADP_Stopped_ApplicationExit → process exit 0. */
__attribute__((noreturn)) void _exit(int status) {
    register uint32_t r0 __asm("r0") = 0x18u; /* SYS_EXIT */
    register uint32_t r1 __asm("r1") =
        (status == 0) ? 0x20026u   /* ADP_Stopped_ApplicationExit */
                      : 0x20023u;  /* ADP_Stopped_RunTimeError    */
    __asm volatile("bkpt 0xab" : : "r"(r0), "r"(r1));
    for (;;) {} /* unreachable */
}
