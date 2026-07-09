// Shared SRT socket configuration for the example caller. LIVE transmission mode
// (the libsrt default) so it interoperates with the host tst-srt listener,
// which is a LIVE-streaming library (TSBPD + message API on; no FILE-mode
// listener knob). Over the lossless SLIRP path LIVE delivers the golden
// byte-exact (no too-late-packet-drop fires without loss). Phase B additionally
// sets a passphrase (compile with -DSRT_PASSPHRASE="...").
#ifndef SRT_OPTS_H
#define SRT_OPTS_H
#include <srt/srt.h>

static inline int srt_apply_opts(SRTSOCKET s) {
    // LIVE is the default transtype; setting it explicitly documents intent and
    // keeps both ends matched (a FILE caller vs a LIVE listener is rejected at
    // handshake — TSBPD/message-API negotiation mismatch). SRTO_TRANSTYPE takes
    // an SRT_TRANSTYPE value, NOT int: arm-none-eabi defaults to -fshort-enums
    // so sizeof(SRT_TRANSTYPE) == 1; passing an int (size 4) fails libsrt's
    // cast_optval size check (-> MN_INVAL throw).
#ifdef SRT_FILE_MODE
    SRT_TRANSTYPE tt = SRTT_FILE;   /* loopback-arq: reliable byte-exact under loss */
#else
    SRT_TRANSTYPE tt = SRTT_LIVE;   /* example: interop with the LIVE tst-srt host listener */
#endif
    if (srt_setsockflag(s, SRTO_TRANSTYPE, &tt, sizeof tt) == SRT_ERROR) return -1;
    // Modest buffers (keep SRT's allocations small on the 1 MiB FreeRTOS heap).
    int buf = 256 * 1024;
    if (srt_setsockflag(s, SRTO_SNDBUF, &buf, sizeof buf) == SRT_ERROR) return -1;
    if (srt_setsockflag(s, SRTO_RCVBUF, &buf, sizeof buf) == SRT_ERROR) return -1;
#ifdef SRT_PASSPHRASE
    // TEST-ONLY key material: the passphrase is a compile-time constant and the
    // entropy behind the derived keys is a deterministic LCG (substrate/
    // syscalls_stub.c; see the README "Production crypto warning"). Do not copy
    // this setup into production firmware.
    const char* pp = SRT_PASSPHRASE;
    int klen = 16;  // AES-128
    if (srt_setsockflag(s, SRTO_PASSPHRASE, pp, (int)__builtin_strlen(pp)) == SRT_ERROR) return -1;
    if (srt_setsockflag(s, SRTO_PBKEYLEN, &klen, sizeof klen) == SRT_ERROR) return -1;
#endif
    return 0;
}
#endif
