/* Cortex-M4F startup for QEMU mps2-an386 + newlib/rdimon semihosting + FreeRTOS.
 *
 * Adapted from crates/baremetal-qemu-c/firmware/startup.c. The only structural
 * change for FreeRTOS is the vector table: the kernel's ARM_CM4F port installs
 * the context-switch + tick machinery via the SVC, PendSV and SysTick exception
 * vectors. Their handler symbols are vPortSVCHandler / xPortPendSVHandler /
 * xPortSysTickHandler (see vendor/freertos-kernel/portable/GCC/ARM_CM4F/port.c).
 * We wire them into vector slots 11 (SVCall), 14 (PendSV) and 15 (SysTick).
 *
 * Root-cause notes carried over from c-1:
 *   - __libc_init_array() calls _init() first; _init is assembled from
 *     crti.o (prologue) + crtn.o (epilogue). The linker script must sequence
 *     KEEP(*crti.o(.init)) … KEEP(*crtn.o(.init)) or _init never returns.
 *   - _exit() is overridden with a raw SYS_EXIT semihosting BKPT for a clean,
 *     newlib-version-independent QEMU process exit.
 */
#include <stdint.h>
#include <errno.h>

extern uint32_t _sidata, _sdata, _edata, _sbss, _ebss, _estack;
extern char end, _heap_end;
extern int  main(void);
extern void __libc_init_array(void);
extern void initialise_monitor_handles(void); /* rdimon */

/* FreeRTOS ARM_CM4F port handlers (defined in vendor port.c). */
extern void vPortSVCHandler(void);
extern void xPortPendSVHandler(void);
extern void xPortSysTickHandler(void);

/* Forward declaration — defined at the bottom of this file. */
__attribute__((noreturn)) void _exit(int status);

void Reset_Handler(void);
static void Default_Handler(void) { for (;;) {} }

/* Cortex-M vector table. Slots 0..15 are the system exceptions; we only need
   the system ones FreeRTOS uses plus reset/NMI/HardFault. Intervening slots
   are padded with Default_Handler so the indices line up. */
__attribute__((section(".isr_vector"), used))
void (* const g_vectors[])(void) = {
    (void (*)(void))(&_estack), /* [0]  initial MSP        */
    Reset_Handler,              /* [1]  Reset              */
    Default_Handler,            /* [2]  NMI                */
    Default_Handler,            /* [3]  HardFault          */
    Default_Handler,            /* [4]  MemManage          */
    Default_Handler,            /* [5]  BusFault           */
    Default_Handler,            /* [6]  UsageFault         */
    0,                          /* [7]  reserved           */
    0,                          /* [8]  reserved           */
    0,                          /* [9]  reserved           */
    0,                          /* [10] reserved           */
    vPortSVCHandler,            /* [11] SVCall             */
    Default_Handler,            /* [12] DebugMon           */
    0,                          /* [13] reserved           */
    xPortPendSVHandler,         /* [14] PendSV             */
    xPortSysTickHandler,        /* [15] SysTick            */
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

/* Self-contained heap for newlib malloc / the C++ runtime. */
void *_sbrk(int incr) {
    static char *brk = 0;
    if (!brk) brk = &end;
    if (brk + incr > &_heap_end || brk + incr < &end) {
        errno = ENOMEM;
        return (void *)-1;
    }
    char *prev = brk; brk += incr; return prev;
}

/* Override newlib's _exit() with a direct ARM semihosting SYS_EXIT BKPT.
 *   R0 = 0x18 (SYS_EXIT)
 *   R1 = ADP_Stopped_ApplicationExit (0x20026) for exit(0),
 *        ADP_Stopped_RunTimeError    (0x20023) for any non-zero status.
 * QEMU maps ADP_Stopped_ApplicationExit → process exit 0. */
__attribute__((noreturn)) void _exit(int status) {
    register uint32_t r0 __asm("r0") = 0x18u; /* SYS_EXIT */
    register uint32_t r1 __asm("r1") =
        (status == 0) ? 0x20026u   /* ADP_Stopped_ApplicationExit */
                      : 0x20023u;  /* ADP_Stopped_RunTimeError    */
    __asm volatile("bkpt 0xab" : : "r"(r0), "r"(r1));
    for (;;) {} /* unreachable */
}
