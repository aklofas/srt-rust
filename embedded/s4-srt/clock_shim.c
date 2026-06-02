/* Hi-res clock_gettime(CLOCK_MONOTONIC) for the S1 harness (R3).
 *
 * FreeRTOS-Plus-POSIX's FreeRTOS_POSIX_clock.c provides a tick-resolution
 * (~1 ms) clock_gettime AND several other TUs (pthread_cond/mutex, timer,
 * semaphore, mqueue) call it — so we can't just redefine the symbol (two strong
 * defs = multiple-definition) nor drop that TU (it also owns nanosleep/
 * clock_nanosleep/clock_getres). Instead the link uses `-Wl,--wrap=clock_gettime`:
 * every reference to clock_gettime (ours + the POSIX TUs' + std::chrono later)
 * resolves to __wrap_clock_gettime below, while the wrapper's own def survives
 * only as the now-unreferenced __real_clock_gettime. This is the S1 analog of
 * S0's strong-symbol override for __cxa_get_globals — same goal (our hi-res def
 * wins the link), cleaner mechanism for an inter-TU symbol.
 *
 * It subdivides each tick with the SysTick current-value register.
 *
 *   ns = tick * 1e6  +  ((reload - SYST_CVR) * 1e9) / configCPU_CLOCK_HZ
 *
 * With configCPU_CLOCK_HZ = 25 MHz and configTICK_RATE_HZ = 1000 the reload is
 * 25000, so the CVR subdivides 1 ms to ~40 ns. SysTick is modeled by QEMU
 * (FreeRTOS drives it). A re-read guards against a tick rollover landing
 * between the tick read and the CVR read.
 */
#include <time.h>
#include <stdint.h>
#include "FreeRTOS.h"
#include "task.h"

#define SYST_RVR  (*(volatile uint32_t *)0xE000E014u)  /* reload value   */
#define SYST_CVR  (*(volatile uint32_t *)0xE000E018u)  /* current value  */

int __wrap_clock_gettime(clockid_t clock_id, struct timespec *tp) {
    (void)clock_id;
    if (tp == 0) return -1;

    uint32_t reload = SYST_RVR + 1u;  /* counts per tick (25000) */

    /* Lock-free two-sample read: pair a SysTick CVR sample with a tick value
     * proven stable across it. A SysTick wrap fires the (enabled) tick ISR
     * essentially atomically, so if the tick read on either side of the CVR
     * sample agree, that CVR belongs to that tick; if they differ, a wrap
     * happened and the SECOND pair (taken after the ISR bumped the tick) is the
     * consistent one. This is monotonic across calls (a wrap advances the tick
     * before the CVR resets), unlike a tick/CSR/CVR three-read sequence where a
     * wrap landing mid-sequence makes the timestamp jump backward. */
    uint32_t m0 = (uint32_t)xTaskGetTickCount();
    uint32_t v0 = SYST_CVR;
    uint32_t m1 = (uint32_t)xTaskGetTickCount();
    uint32_t v1 = SYST_CVR;

    uint32_t tick, into_tick;
    if (m0 == m1) { tick = m0; into_tick = reload - v0; }
    else          { tick = m1; into_tick = reload - v1; }

    uint64_t ns = (uint64_t)tick * 1000000ull
                + ((uint64_t)into_tick * 1000000000ull) / configCPU_CLOCK_HZ;
    tp->tv_sec  = (time_t)(ns / 1000000000ull);
    tp->tv_nsec = (long)(ns % 1000000000ull);
    return 0;
}
