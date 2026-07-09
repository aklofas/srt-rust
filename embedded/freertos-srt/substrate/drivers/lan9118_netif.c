/* Register-level polling driver for QEMU's lan9118 model on MPS2 (an386).
 * Targets QEMU's subset, not silicon: link is always up, no PHY autoneg, no
 * IRQ. TX writes command words A/B + data to TX_DATA_FIFO; RX reads the
 * RX_STATUS_FIFO for the frame length then drains RX_DATA_FIFO. MAC is set
 * promiscuous (PRMS) so frame filtering never gets in the way of the test. */
#include "lwip/opt.h"
#include "lwip/netif.h"
#include "lwip/tcpip.h"
#include "lwip/etharp.h"
#include "lwip/ethip6.h"
#include "lwip/snmp.h"
#include "netif/ethernet.h"
#include "lan9118_netif.h"
#include "diag.h"

#define LAN9118_BASE 0x40200000u
#define REG(off)     (*(volatile uint32_t *)(LAN9118_BASE + (off)))

/* Directly-addressable registers (offsets per SMSC LAN9118 datasheet). */
#define RX_DATA_FIFO 0x00
#define TX_DATA_FIFO 0x20
#define RX_STATUS_FIFO 0x40
#define ID_REV       0x50
#define IRQ_CFG      0x54
#define INT_STS      0x58
#define INT_EN       0x5C
#define BYTE_TEST    0x64
#define FIFO_INT     0x68
#define RX_CFG       0x6C
#define TX_CFG       0x70
#define HW_CFG       0x74
#define RX_FIFO_INF  0x7C
#define TX_FIFO_INF  0x80
#define MAC_CSR_CMD  0xA4
#define MAC_CSR_DATA 0xA8

/* TX_CFG / MAC_CSR / MAC_CR bits. */
#define TX_CFG_TX_ON   0x00000002u
#define MAC_CSR_BUSY   0x80000000u
#define MAC_CSR_READ   0x40000000u
#define MAC_CR         0x01u
#define MAC_ADDRH      0x02u
#define MAC_ADDRL      0x03u
#define MAC_CR_TXEN    0x00000008u
#define MAC_CR_RXEN    0x00000004u
#define MAC_CR_PRMS    0x00040000u   /* promiscuous */

static struct netif s_netif;
static const uint8_t s_mac[6] = {0x52,0x54,0x00,0x12,0x34,0x56};

static void mac_write(uint32_t idx, uint32_t val) {
    REG(MAC_CSR_DATA) = val;
    REG(MAC_CSR_CMD)  = MAC_CSR_BUSY | idx;          /* write (READ bit clear) */
    while (REG(MAC_CSR_CMD) & MAC_CSR_BUSY) { }
}

/* TX: one segment = whole frame. Command A: first+last seg, data-start offset 0,
 * buffer size = len. Command B: packet length + packet tag. Then the data,
 * word-aligned, little-endian (QEMU treats the FIFO as 32-bit words). */
static err_t low_level_output(struct netif *n, struct pbuf *p) {
    (void)n;
    uint32_t len = p->tot_len;
    /* Word-typed staging buffer: the FIFO is pushed 32 bits at a time, and a
     * uint8_t buffer punned through uint32_t* is undefined behavior under
     * strict aliasing (-Os enables it). Byte-copies INTO a uint32_t array via
     * pbuf_copy_partial/memcpy are always legal; the reverse pun is not. */
    static uint32_t buf[1600 / 4];
    /* Reject oversize frames BEFORE any FIFO write: the two command words
     * would otherwise already be queued, corrupting TX framing for the next
     * packet instead of cleanly returning ERR_BUF. */
    if (len > sizeof buf) return ERR_BUF;
    REG(TX_DATA_FIFO) = (1u << 13) | (1u << 12) | (len & 0x7FFu);   /* cmd A: FS|LS|size */
    REG(TX_DATA_FIFO) = (len & 0xFFFFu);                            /* cmd B: length+tag */
    pbuf_copy_partial(p, buf, len, 0);
    uint32_t words = (len + 3u) / 4u;
    for (uint32_t i = 0; i < words; i++) REG(TX_DATA_FIFO) = buf[i];
    MIB2_STATS_NETIF_ADD(n, ifoutoctets, len);
    return ERR_OK;
}

void lan9118_poll(void) {
    /* Drain EVERY queued frame per poll: RX_FIFO_INF bits 23:16 = status FIFO
     * used count — loop until it reads empty. */
    while (((REG(RX_FIFO_INF) >> 16) & 0xFFu) != 0) {
        uint32_t status = REG(RX_STATUS_FIFO);
        uint32_t len = (status >> 16) & 0x3FFFu;         /* length incl CRC */
        static uint32_t buf[1600 / 4];                    /* word-typed: see TX */
        uint32_t words = (len + 3u) / 4u;
        struct pbuf *p = (len == 0 || len > sizeof buf) ? NULL
                         : pbuf_alloc(PBUF_RAW, (u16_t)len, PBUF_RAM);
        if (p == NULL) {
            for (uint32_t i = 0; i < words; i++) (void)REG(RX_DATA_FIFO);
            continue;
        }
        for (uint32_t i = 0; i < words; i++) buf[i] = REG(RX_DATA_FIFO);
        pbuf_take(p, buf, (u16_t)len);
        if (s_netif.input(p, &s_netif) != ERR_OK) pbuf_free(p);
    }
}

static err_t lan9118_init(struct netif *netif) {
    netif->name[0] = 'e'; netif->name[1] = 'n';
    netif->output     = etharp_output;
#if LWIP_IPV6
    netif->output_ip6 = ethip6_output;
#endif
    netif->linkoutput = low_level_output;
    netif->mtu        = 1500;
    netif->hwaddr_len = 6;
    for (int i = 0; i < 6; i++) netif->hwaddr[i] = s_mac[i];
    netif->flags = NETIF_FLAG_BROADCAST | NETIF_FLAG_ETHARP | NETIF_FLAG_LINK_UP;

    /* Endianness probe: QEMU must return 0x87654321 from BYTE_TEST. A probe
     * that discards the value cannot fail — assert it loudly, since every
     * later FIFO word access assumes this little-endian register mapping. */
    if (REG(BYTE_TEST) != 0x87654321u) tst_diag_fail("lan9118_byte_test");
    /* HW_CFG: leave at reset defaults. The previous `REG(HW_CFG) = 0` zeroed
     * the TX-FIFO-size field it claimed to leave alone. */
    /* Program MAC address: ADDRL = low 4 bytes, ADDRH = high 2 bytes. */
    mac_write(MAC_ADDRL, s_mac[0] | (s_mac[1]<<8) | (s_mac[2]<<16) | (s_mac[3]<<24));
    mac_write(MAC_ADDRH, s_mac[4] | (s_mac[5]<<8));
    mac_write(MAC_CR, MAC_CR_TXEN | MAC_CR_RXEN | MAC_CR_PRMS);
    REG(TX_CFG) = TX_CFG_TX_ON;
    return ERR_OK;
}

void lan9118_netif_up(uint8_t a, uint8_t b, uint8_t c, uint8_t d,
                      uint8_t ga, uint8_t gb, uint8_t gc, uint8_t gd) {
    ip4_addr_t ip, mask, gw;
    IP4_ADDR(&ip, a, b, c, d);
    IP4_ADDR(&mask, 255, 255, 255, 0);
    IP4_ADDR(&gw, ga, gb, gc, gd);
    netif_add(&s_netif, &ip, &mask, &gw, NULL, lan9118_init, tcpip_input);
    netif_set_default(&s_netif);
    netif_set_up(&s_netif);
    netif_set_link_up(&s_netif);
}
