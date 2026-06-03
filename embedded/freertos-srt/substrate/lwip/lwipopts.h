/* lwIP config for the libsrt-smoke harness: a single device, loopback netif only, UDP +
 * BSD sockets + select(). No NIC, no ARP/ethernet, no TCP. IPv6 is ENABLED (see
 * LWIP_IPV6 below) so libsrt's sockaddr_in6/ip6_addr_t types resolve, but only
 * IPv4 loopback traffic actually flows. The minimum surface libsrt's
 * channel.cpp (sendto/recvfrom) + CEPoll (select) exercise. */
#ifndef LWIP_LWIPOPTS_H
#define LWIP_LWIPOPTS_H

#include <stdlib.h>   /* rand() for LWIP_RAND */

#define NO_SYS                      0       /* full OS mode: sockets + tcpip thread */
#define SYS_LIGHTWEIGHT_PROT        1
#define LWIP_TIMERS                 1
#define LWIP_RAND()                 ((u32_t)rand())

/* Protocols: UDP over IPv4 only. */
#define LWIP_IPV4                   1
/* libsrt-smoke: libsrt references sockaddr_in6 / ip6_addr_t / IN6_IS_ADDR_* / IPV6_*
 * unconditionally (it's IPv6-capable). lwIP only defines them under LWIP_IPV6.
 * Enable it so libsrt compiles against real lwIP IPv6 types (the boot smoke and
 * the loopback-arq SRT loopback still run over IPv4 — no IPv6 traffic). Adds the
 * core/ipv6 *.c sources to build.sh. */
#define LWIP_IPV6                   1
#define LWIP_UDP                    1
#define LWIP_TCP                    0
#define LWIP_RAW                    0
#define LWIP_DHCP                   0
/* libsrt-smoke: libsrt's channel.cpp calls ::getaddrinfo(NULL,"0",...) to resolve the
 * wildcard bind address, and netinet_any.h needs `struct addrinfo`. lwIP only
 * exposes getaddrinfo + struct addrinfo (lwip/netdb.h) under LWIP_DNS. dns.c +
 * api/netdb.c are already in the source glob; no actual DNS runs in the smoke. */
#define LWIP_DNS                    1
/* example: real lan9118 Ethernet (not the loopback-arq loopback netif). The driver hands raw
 * frames to lwIP's ethernet_input, so ARP (to resolve the SLIRP gateway
 * 10.0.2.2) and the ethernet layer must be on. */
#define LWIP_ARP                    1
#define LWIP_ETHERNET               1
#define LWIP_IGMP                   0
#define LWIP_ICMP                   1

/* Superset base: Ethernet+ARP (for the lan9118 example) AND a built-in-loopif
 * path (for the lwip-loopback test) both compiled in. The loopback toggles are
 * #ifndef-guarded so the lwip-loopback target re-enables them via -D without a
 * forked config; loopback-arq + example leave them at 0 (the lossy/lan9118
 * netifs own their addresses and must see every packet). */
#ifndef LWIP_HAVE_LOOPIF
#define LWIP_HAVE_LOOPIF            0
#endif
#ifndef LWIP_NETIF_LOOPBACK
#define LWIP_NETIF_LOOPBACK         0
#endif
#ifndef LWIP_NETIF_LOOPBACK_MULTITHREADING
#define LWIP_NETIF_LOOPBACK_MULTITHREADING 0
#endif

/* Sequential / socket API. */
#define LWIP_NETCONN                1
#define LWIP_SOCKET                 1
#define LWIP_SOCKET_SELECT          1
/* libsrt-smoke: libsrt's channel.cpp/epoll.cpp call UNPREFIXED BSD names (socket/bind/
 * select/...) and the UDT namespace declares its OWN socket()/bind()/sendmsg()
 * API. Mode 2 exports the bare names as REAL functions (`#define lwip_bind bind`
 * + real `int bind(...)`), NOT function-like macros — so they satisfy libsrt's
 * ::socket()/::bind() calls at link time WITHOUT clobbering UDT::bind or
 * std::bind (mode 1's macros break both; lwip-loopback used lwip_* explicitly with =0). */
#define LWIP_COMPAT_SOCKETS         2
#define LWIP_POSIX_SOCKETS_IO_NAMES 0
/* loopback-arq: libsrt's CChannel::setUDPSockOpt sets SO_RCVTIMEO + SO_SNDTIMEO on the UDP
 * socket (its non-blocking-via-short-timeout path on non-UNIX/_WIN32 systems)
 * and throws MJ_SETUP if either setsockopt returns -1. Enable both so lwIP
 * accepts them. (SO_RCVBUF/SO_SNDBUF are handled by the net_shim.c link-wrap.) */
#define LWIP_SO_RCVTIMEO            1
#define LWIP_SO_SNDTIMEO            1
#define LWIP_NETCONN_SEM_PER_THREAD 0

/* Memory — generous; this is an H7-class budget and the traffic is one 564B
 * datagram on loopback. */
#define MEM_ALIGNMENT               4
#define MEM_SIZE                    (64 * 1024)
#define MEMP_NUM_PBUF               32
#define MEMP_NUM_UDP_PCB            8
#define MEMP_NUM_NETCONN            8
#define MEMP_NUM_NETBUF             8
#define MEMP_NUM_TCPIP_MSG_API      16
#define MEMP_NUM_TCPIP_MSG_INPKT    16
#define PBUF_POOL_SIZE              16

/* ARP table + a couple of queued packets while ARP resolves the gateway. */
#define MEMP_NUM_ARP_QUEUE          5
#define ARP_TABLE_SIZE              4
#define ARP_QUEUEING                1
#define ETH_PAD_SIZE                0

/* Threading: the tcpip thread + socket mboxes (sized via our sys_arch). */
#define TCPIP_THREAD_NAME           "tcpip"
#define TCPIP_THREAD_STACKSIZE      2048
#define TCPIP_THREAD_PRIO           4
#define TCPIP_MBOX_SIZE             16
#define DEFAULT_UDP_RECVMBOX_SIZE   16
#define DEFAULT_ACCEPTMBOX_SIZE     8
#define DEFAULT_THREAD_STACKSIZE    2048

/* Stats/debug off for footprint. */
#define LWIP_STATS                  0
#define LWIP_NETIF_API              1
#define LWIP_DEBUG                  0

/* errno: use newlib's. lwip/errno.h gates on `#ifdef LWIP_PROVIDE_ERRNO`
 * (existence, NOT value), so defining it to 0 would STILL make lwIP define its
 * own ETIMEDOUT=110 etc., clashing with newlib's <errno.h> (ETIMEDOUT=116) and
 * flooding the build with macro-redefinition warnings. Instead leave
 * LWIP_PROVIDE_ERRNO undefined and tell lwip/errno.h to pull in <errno.h>. */
#define LWIP_ERRNO_STDINCLUDE       1

/* Use the system struct timeval (from <sys/time.h>) rather than lwIP's private
 * one — once main.cpp also pulls the FreeRTOS-Plus-POSIX/newlib headers, lwIP's
 * own definition collides (redefinition of 'struct timeval'). */
#define LWIP_TIMEVAL_PRIVATE        0

#endif /* LWIP_LWIPOPTS_H */
