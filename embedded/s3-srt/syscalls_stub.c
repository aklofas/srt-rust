/* Bare-metal syscall stubs needed once libsrt is linked.
 *
 * _getentropy: libsrt seeds initial sequence numbers / socket IDs via
 * std::random_device, which on newlib funnels through getentropy() -> the
 * _getentropy syscall. Bare metal has no entropy backend, so it is unresolved.
 * S2 builds with encryption OFF and never needs cryptographic randomness, so a
 * deterministic LCG is enough to link and boot. This is NOT suitable for real
 * crypto — a production embedded build must wire a hardware RNG here. */
#include <stddef.h>
#include <stdint.h>
#include <errno.h>
#include <time.h>
#include <sys/time.h>

int _getentropy(void* buf, size_t n)
{
    static uint32_t seed = 0x01234567u;
    uint8_t* p = (uint8_t*)buf;
    if (p == NULL && n != 0) { errno = EFAULT; return -1; }  /* fail cleanly */
    for (size_t i = 0; i < n; i++) {
        seed = seed * 1103515245u + 12345u;
        p[i] = (uint8_t)(seed >> 16);
    }
    return 0;
}

/* __wrap_gettimeofday: with ENABLE_MONOTONIC_CLOCK=OFF, libsrt's sync_posix
 * Condition::wait_for builds its pthread_cond_timedwait abstime from
 * gettimeofday(). FreeRTOS-Plus-POSIX's pthread_cond_timedwait, however, derives
 * its wait delay from clock_gettime(CLOCK_REALTIME). The two MUST share a time
 * base or every timed wait computes a negative/garbage delay and returns
 * instantly -> SRT's GC + sender-pacing + retransmit threads busy-spin (they
 * starve the receive workers; the SRT handshake then never completes). newlib's
 * gettimeofday (rdimon semihosting) is unrelated to our tick clock, so the link
 * wraps gettimeofday (-Wl,--wrap=gettimeofday): SRT's call routes here and we
 * return the SAME hi-res clock the clock_gettime --wrap returns (the call below
 * resolves to __wrap_clock_gettime via that link wrap). Clock id is ignored by
 * __wrap_clock_gettime; declared explicitly since newlib gates CLOCK_MONOTONIC. */
extern int clock_gettime(int clk, struct timespec* ts);

int __wrap_gettimeofday(struct timeval* tv, void* tz)
{
    (void)tz;
    if (tv) {
        struct timespec ts;
        clock_gettime(1, &ts);                 /* -> __wrap_clock_gettime */
        tv->tv_sec  = ts.tv_sec;
        tv->tv_usec = ts.tv_nsec / 1000;
    }
    return 0;
}
