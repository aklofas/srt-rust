/* Minimal polling lwIP driver for QEMU's lan9118 (MPS2 ethernet @ 0x40200000). */
#ifndef LAN9118_NETIF_H
#define LAN9118_NETIF_H
#include <stdint.h>
/* Bring up the lan9118 netif at the given static IPv4 (host byte order args). */
void lan9118_netif_up(uint8_t a, uint8_t b, uint8_t c, uint8_t d,   /* ip */
                      uint8_t ga, uint8_t gb, uint8_t gc, uint8_t gd); /* gw */
/* Poll the RX FIFO once and push any received frame into lwIP. Call in a loop
 * from a dedicated FreeRTOS task. */
void lan9118_poll(void);
#endif
