/* libsrt's platform_sys.h includes <netdb.h>. lwIP provides getaddrinfo + struct
 * addrinfo (in lwip/netdb.h, under LWIP_DNS — which we enable in lwipopts), but
 * NO getnameinfo and none of the NI_* flags. Forward to lwIP's netdb and supply
 * the missing pieces. */
#ifndef S2_SHIM_NETDB_H
#define S2_SHIM_NETDB_H

#include <sys/socket.h>
#include <lwip/netdb.h>

/* NI_* flags (libsrt's sockaddr_any::str() uses NI_NUMERICHOST/NUMERICSERV/
 * NAMEREQD). Standard POSIX values. */
#ifndef NI_NUMERICHOST
#define NI_MAXHOST     1025
#define NI_MAXSERV     32
#define NI_NUMERICHOST 0x01
#define NI_NUMERICSERV 0x02
#define NI_NOFQDN      0x04
#define NI_NAMEREQD    0x08
#define NI_DGRAM       0x10
#endif

/* lwIP has no getnameinfo. S2 does no reverse-name resolution (the SRT data
 * plane uses numeric addresses), so a stub that fails is sufficient: libsrt's
 * sockaddr_any::str() treats a nonzero return as "no host" and falls back to
 * ":<port>". Real impl deferred — not needed until/unless a binding wants it. */
#ifndef EAI_FAIL
#define EAI_FAIL (-4)   /* nonzero; POSIX getnameinfo returns 0 or an EAI_* code */
#endif
static inline int getnameinfo(const struct sockaddr* sa, socklen_t salen,
                              char* host, socklen_t hostlen,
                              char* serv, socklen_t servlen, int flags) {
    (void)sa; (void)salen; (void)flags;
    if (host && hostlen) host[0] = '\0';   /* don't leave caller buffers unset */
    if (serv && servlen) serv[0] = '\0';
    return EAI_FAIL;
}

#endif
