/* lwIP defines sockaddr_in/in_addr/IPPROTO_* in lwip/sockets.h, pulled via the
 * compat <sys/socket.h>; lwIP ships no <netinet/in.h>, so forward to it. */
#ifndef FREERTOS_SRT_SHIM_NETINET_IN_H
#define FREERTOS_SRT_SHIM_NETINET_IN_H
#include <sys/socket.h>

/* libsrt's netinet_any.h / channel.cpp / api.cpp reference sockaddr_in6 +
 * IPPROTO_IPV6 + IPV6_* unconditionally (it supports IPv6 at compile time). lwIP
 * already defines `struct in6_addr` (+ s6_addr) in lwip/inet.h, and — because
 * our lwipopts.h enables LWIP_IPV6 — `struct sockaddr_in6` too. The `#if
 * !LWIP_IPV6` block below is therefore an INACTIVE fallback, retained only for
 * an IPv4-only configuration; with IPv6 enabled it compiles out. The IPV6_*
 * socket-option gap-fills further down stay active unconditionally because lwIP
 * doesn't define all of them (e.g. IPV6_UNICAST_HOPS). Cosmetic at runtime — no
 * AF_INET6 socket is ever created. */
#if !LWIP_IPV6
#include <stdint.h>
#ifndef AF_INET6
#define AF_INET6 10              /* matches lwIP lwip/sockets.h value */
#endif
struct sockaddr_in6 {
    uint8_t     sin6_len;
    sa_family_t sin6_family;
    in_port_t   sin6_port;
    uint32_t    sin6_flowinfo;
    struct in6_addr sin6_addr;   /* lwIP lwip/inet.h provides in6_addr */
    uint32_t    sin6_scope_id;
};
#endif /* !LWIP_IPV6 */

/* IPv6 socket-option names libsrt sets on AF_INET6 sockets. lwIP defines some
 * (IPPROTO_IPV6, IPV6_V6ONLY) under LWIP_IPV6 but NOT IPV6_UNICAST_HOPS, and
 * none with IPv6 off. Fill the gaps with the standard Linux values (#ifndef so
 * lwIP's own defs win where present). Cosmetic here — no AF_INET6 socket is
 * created at runtime in the IPv4-only boot smoke / loopback-arq loopback. */
#ifndef IPPROTO_IPV6
#define IPPROTO_IPV6      41
#endif
#ifndef IPV6_UNICAST_HOPS
#define IPV6_UNICAST_HOPS 16
#endif
#ifndef IPV6_V6ONLY
#define IPV6_V6ONLY       26
#endif

#endif
