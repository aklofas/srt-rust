/* setsockopt link-wrap for the bare-metal lwIP UDP channel.
 *
 * Two libsrt-on-lwIP impedance mismatches handled here via `-Wl,--wrap=setsockopt`
 * (LWIP_COMPAT_SOCKETS=2 compiles lwip_setsockopt AS `setsockopt`, so every
 * setsockopt() call routes through __wrap_setsockopt; __real_setsockopt is lwIP's):
 *
 * 1. SO_RCVBUF / SO_SNDBUF: lwIP doesn't implement SO_SNDBUF (ENOPROTOOPT) and
 *    gates SO_RCVBUF behind LWIP_SO_RCVBUF; libsrt's CChannel::setUDPSockOpt()
 *    THROWS MJ_SETUP if either returns -1. lwIP buffers UDP through its global
 *    PBUF/MEMP pools (sized in lwipopts.h), not per-socket, so these are genuine
 *    no-ops here: swallow them.
 *
 * 2. SO_RCVTIMEO / SO_SNDTIMEO: libsrt sets a 100us timeout so its CRcvQueue
 *    worker's blocking recvfrom() returns promptly to service ARQ/handshake
 *    timers. lwIP's SO_*TIMEO takes a `struct timeval` it converts to whole
 *    milliseconds, and treats 0ms as INFINITE — so 100us rounds to 0 -> the
 *    worker blocks until the NEXT datagram arrives and never services timers on
 *    time (the SRT handshake response lags a whole resend interval -> connect
 *    times out). Floor any non-zero sub-millisecond timeout to 1ms.
 *
 * Everything else delegates to lwIP, so genuine option errors still surface.
 * Same link-wrap mechanism as the clock_gettime hi-res override. */
#include "lwip/sockets.h"
#include <sys/time.h>

extern int __real_setsockopt(int s, int level, int optname,
                             const void *optval, socklen_t optlen);

int __wrap_setsockopt(int s, int level, int optname,
                      const void *optval, socklen_t optlen)
{
    if (level == SOL_SOCKET && (optname == SO_RCVBUF || optname == SO_SNDBUF))
        return 0;   /* lwIP manages UDP buffering globally; nothing per-socket */

    if (level == SOL_SOCKET && (optname == SO_RCVTIMEO || optname == SO_SNDTIMEO)
        && optval != NULL && optlen >= sizeof(struct timeval)) {
        const struct timeval *tv = (const struct timeval *)optval;
        long ms = (long)tv->tv_sec * 1000 + tv->tv_usec / 1000;
        if (ms < 1 && (tv->tv_sec != 0 || tv->tv_usec != 0)) {
            static const struct timeval one_ms = { 0, 1000 };   /* 1ms floor */
            return __real_setsockopt(s, level, optname, &one_ms, sizeof one_ms);
        }
    }

    return __real_setsockopt(s, level, optname, optval, optlen);
}
