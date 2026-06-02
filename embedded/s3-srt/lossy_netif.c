/* A hand-written lwIP loopback netif with deterministic packet loss.
 *
 * Replaces lwIP's built-in loopback (LWIP_NETIF_LOOPBACK=0 in lwipopts). The
 * netif sits at 127.0.0.1; its output loops a COPY of each pbuf back up the
 * stack via netif->input (tcpip_input). A datagram is dropped only when the
 * filter is enabled AND it is a DATA-sized packet (tot_len > 900) AND it is the
 * 5th such packet (~20%). SRT control/handshake/KM packets are small and always
 * pass, so the connection and keying are never disrupted. Deterministic ->
 * the CI gate reproduces. Self-converging: a dropped packet's retransmit is a
 * new datagram with an independent ~20% drop chance, so it gets through after a
 * few tries; byte-exact recovery is guaranteed. */
#include "lwip/opt.h"
#include "lwip/netif.h"
#include "lwip/tcpip.h"
#include "lwip/ip4_addr.h"
#include "lwip/pbuf.h"
#include "lossy_netif.h"

static struct netif      s_netif;
static volatile int      s_enabled = 1;
static volatile uint32_t s_data_ord = 0;
static volatile uint32_t s_dropped = 0;

void     lossy_set_enabled(int en) { s_enabled = en; }
uint32_t lossy_dropped_count(void) { return s_dropped; }

/* Loop a copy of p back up the stack as if it arrived on this netif. */
static err_t loop_back(struct netif *netif, struct pbuf *p)
{
    struct pbuf *q = pbuf_alloc(PBUF_RAW, p->tot_len, PBUF_RAM);
    if (q == NULL) return ERR_MEM;
    if (pbuf_copy(q, p) != ERR_OK) { pbuf_free(q); return ERR_BUF; }
    if (netif->input(q, netif) != ERR_OK) { pbuf_free(q); return ERR_IF; }
    return ERR_OK;
}

static err_t lossy_linkoutput(struct netif *netif, struct pbuf *p)
{
    if (s_enabled && p->tot_len > 900) {            /* data packet, not control */
        uint32_t ord = ++s_data_ord;
        if (ord % 5 == 0) { s_dropped++; return ERR_OK; }   /* silently drop */
    }
    return loop_back(netif, p);
}

static err_t lossy_output_v4(struct netif *netif, struct pbuf *p,
                             const ip4_addr_t *ipaddr)
{
    (void)ipaddr;                                   /* loopback: no ARP */
    return lossy_linkoutput(netif, p);
}

static err_t lossy_netif_init(struct netif *netif)
{
    netif->name[0] = 'l'; netif->name[1] = 'o';
    netif->output      = lossy_output_v4;
    netif->linkoutput  = lossy_linkoutput;
    netif->mtu         = 1500;
    netif->flags       = NETIF_FLAG_LINK_UP;        /* up; no ARP/broadcast */
    return ERR_OK;
}

void lossy_netif_up(void)
{
    ip4_addr_t ip, mask, gw;
    /* 10.0.0.1/24, NOT 127.0.0.1: lwIP special-cases loopback addresses and,
     * with LWIP_HAVE_LOOPIF=0 (no real loopback netif), packets to 127.0.0.1 are
     * accepted at the IP layer (netif address match) but never matched to the
     * bound UDP pcb -> recv_udp is never called. A normal address on our netif
     * routes + delivers cleanly. Both endpoints use 10.0.0.1 (main.cpp). */
    IP4_ADDR(&ip, 10, 0, 0, 1);
    IP4_ADDR(&mask, 255, 255, 255, 0);
    IP4_ADDR(&gw, 0, 0, 0, 0);
    netif_add(&s_netif, &ip, &mask, &gw, NULL, lossy_netif_init, tcpip_input);
    netif_set_default(&s_netif);
    netif_set_up(&s_netif);
}
