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
