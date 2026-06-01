/* lwIP config for the S1 harness: a single device, loopback netif only, UDP +
 * BSD sockets + select(). No NIC, no ARP/ethernet, no TCP/IPv6 — the minimum
 * surface libsrt's channel.cpp (sendto/recvfrom) + CEPoll (select) exercise. */
#ifndef LWIP_LWIPOPTS_H
#define LWIP_LWIPOPTS_H

#include <stdlib.h>   /* rand() for LWIP_RAND */

#define NO_SYS                      0       /* full OS mode: sockets + tcpip thread */
#define SYS_LIGHTWEIGHT_PROT        1
#define LWIP_TIMERS                 1
#define LWIP_RAND()                 ((u32_t)rand())

/* Protocols: UDP over IPv4 only. */
#define LWIP_IPV4                   1
#define LWIP_IPV6                   0
#define LWIP_UDP                    1
#define LWIP_TCP                    0
#define LWIP_RAW                    0
#define LWIP_DHCP                   0
#define LWIP_DNS                    0
#define LWIP_ARP                    0
#define LWIP_ETHERNET               0
#define LWIP_IGMP                   0
#define LWIP_ICMP                   1

/* Loopback netif (127.0.0.1) — the only interface; no driver. */
#define LWIP_HAVE_LOOPIF            1
#define LWIP_NETIF_LOOPBACK         1
#define LWIP_NETIF_LOOPBACK_MULTITHREADING 1
#define LWIP_LOOPBACK_MAX_PBUFS     8

/* Sequential / socket API. */
#define LWIP_NETCONN                1
#define LWIP_SOCKET                 1
#define LWIP_SOCKET_SELECT          1
#define LWIP_COMPAT_SOCKETS         0
#define LWIP_POSIX_SOCKETS_IO_NAMES 0
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
