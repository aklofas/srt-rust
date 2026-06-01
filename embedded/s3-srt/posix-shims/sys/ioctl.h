/* libsrt uses ioctl(FIONBIO/FIONREAD); lwIP provides lwip_ioctl + these macros
 * via lwip/sockets.h (pulled by the compat <sys/socket.h>). */
#ifndef S2_SHIM_SYS_IOCTL_H
#define S2_SHIM_SYS_IOCTL_H
#include <sys/socket.h>
#endif
