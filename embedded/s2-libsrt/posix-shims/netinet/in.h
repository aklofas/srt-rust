/* lwIP defines sockaddr_in/in_addr/IPPROTO_* in lwip/sockets.h, pulled via the
 * compat <sys/socket.h>; lwIP ships no <netinet/in.h>, so forward to it. */
#ifndef S2_SHIM_NETINET_IN_H
#define S2_SHIM_NETINET_IN_H
#include <sys/socket.h>
#endif
