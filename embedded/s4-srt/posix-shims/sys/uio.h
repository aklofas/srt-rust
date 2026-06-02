/* libsrt's common.h includes <sys/uio.h> for `struct iovec` (scatter/gather in
 * CPacket sendmsg). Bare-metal newlib ships none; lwIP defines `struct iovec`
 * (and lwip_readv/writev) in lwip/sockets.h, pulled via the compat
 * <sys/socket.h>. */
#ifndef S2_SHIM_SYS_UIO_H
#define S2_SHIM_SYS_UIO_H
#include <sys/socket.h>
#endif
