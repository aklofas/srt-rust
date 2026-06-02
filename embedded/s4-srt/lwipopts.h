/* lwIP config for the S2 harness: a single device, loopback netif only, UDP +
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
/* S2: libsrt references sockaddr_in6 / ip6_addr_t / IN6_IS_ADDR_* / IPV6_*
 * unconditionally (it's IPv6-capable). lwIP only defines them under LWIP_IPV6.
 * Enable it so libsrt compiles against real lwIP IPv6 types (the boot smoke and
 * the S3 SRT loopback still run over IPv4 — no IPv6 traffic). Adds the
 * core/ipv6/*.c glob to build.sh. */
#define LWIP_IPV6                   1
#define LWIP_UDP                    1
#define LWIP_TCP                    0
#define LWIP_RAW                    0
#define LWIP_DHCP                   0
/* S2: libsrt's channel.cpp calls ::getaddrinfo(NULL,"0",...) to resolve the
 * wildcard bind address, and netinet_any.h needs `struct addrinfo`. lwIP only
 * exposes getaddrinfo + struct addrinfo (lwip/netdb.h) under LWIP_DNS. dns.c +
 * api/netdb.c are already in the source glob; no actual DNS runs in the smoke. */
#define LWIP_DNS                    1
#define LWIP_ARP                    0
#define LWIP_ETHERNET               0
#define LWIP_IGMP                   0
#define LWIP_ICMP                   1

/* S3: our lossy netif (lossy_netif.c) owns 10.0.0.1 and applies packet loss in
 * its output path, so disable lwIP's built-in loopback short-circuit — otherwise
 * traffic to the netif's own address would bypass our netif and never see the
 * drop filter. (10.0.0.1, not 127.0.0.1: with LWIP_HAVE_LOOPIF=0 lwIP accepts
 * 127/8 at the IP layer but never matches it to the bound UDP pcb.) */
#define LWIP_HAVE_LOOPIF            0
#define LWIP_NETIF_LOOPBACK         0
#define LWIP_NETIF_LOOPBACK_MULTITHREADING 0
#define LWIP_LOOPBACK_MAX_PBUFS     8

/* Sequential / socket API. */
#define LWIP_NETCONN                1
#define LWIP_SOCKET                 1
#define LWIP_SOCKET_SELECT          1
/* S2: libsrt's channel.cpp/epoll.cpp call UNPREFIXED BSD names (socket/bind/
 * select/...) and the UDT namespace declares its OWN socket()/bind()/sendmsg()
 * API. Mode 2 exports the bare names as REAL functions (`#define lwip_bind bind`
 * + real `int bind(...)`), NOT function-like macros — so they satisfy libsrt's
 * ::socket()/::bind() calls at link time WITHOUT clobbering UDT::bind or
 * std::bind (mode 1's macros break both; S1 used lwip_* explicitly with =0). */
#define LWIP_COMPAT_SOCKETS         2
#define LWIP_POSIX_SOCKETS_IO_NAMES 0
/* S3: libsrt's CChannel::setUDPSockOpt sets SO_RCVTIMEO + SO_SNDTIMEO on the UDP
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

/* errno: use newlib's. */
#define LWIP_PROVIDE_ERRNO          0
#include <errno.h>

/* Use the system struct timeval (from <sys/time.h>) rather than lwIP's private
 * one — once main.cpp also pulls the FreeRTOS-Plus-POSIX/newlib headers, lwIP's
 * own definition collides (redefinition of 'struct timeval'). */
#define LWIP_TIMEVAL_PRIVATE        0

#endif /* LWIP_LWIPOPTS_H */
