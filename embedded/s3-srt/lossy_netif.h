#ifndef LOSSY_NETIF_H
#define LOSSY_NETIF_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Bring up a netif at 127.0.0.1 whose output loops packets back to input,
 * deterministically dropping ~20% of DATA-sized packets (control/handshake
 * always pass). Call after tcpip_init() has completed. */
void     lossy_netif_up(void);
/* Master enable for the drop filter (1 = drop ~20% data pkts, 0 = pass all). */
void     lossy_set_enabled(int enabled);
/* How many packets the filter has dropped so far (corroborates srt_bstats). */
uint32_t lossy_dropped_count(void);
#ifdef __cplusplus
}
#endif
#endif
