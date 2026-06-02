// Shared SRT socket configuration for the S3 caller + listener. FILE/buffer
// transmission mode (SRTT_FILE) = SRT's reliable mode: full ARQ until
// delivered, no too-late-packet-drop -> byte-exact recovery under loss. Phase B
// additionally sets a passphrase (compile with -DS3_PASSPHRASE="...").
#ifndef SRT_OPTS_H
#define SRT_OPTS_H
#include <srt/srt.h>

static inline int s3_apply_opts(SRTSOCKET s) {
    // SRTO_TRANSTYPE takes an SRT_TRANSTYPE value, NOT int. The arm-none-eabi
    // EABI defaults to -fshort-enums, so sizeof(SRT_TRANSTYPE) == 1 here; passing
    // an int (size 4) fails libsrt's cast_optval size check (-> MN_INVAL throw).
    SRT_TRANSTYPE tt = SRTT_FILE;
    if (srt_setsockflag(s, SRTO_TRANSTYPE, &tt, sizeof tt) == SRT_ERROR) return -1;
    // Modest buffers (192 KiB FreeRTOS heap — keep SRT's allocations small).
    int buf = 256 * 1024;
    srt_setsockflag(s, SRTO_SNDBUF, &buf, sizeof buf);
    srt_setsockflag(s, SRTO_RCVBUF, &buf, sizeof buf);
#ifdef S3_PASSPHRASE
    const char* pp = S3_PASSPHRASE;
    int klen = 16;  // AES-128
    if (srt_setsockflag(s, SRTO_PASSPHRASE, pp, (int)__builtin_strlen(pp)) == SRT_ERROR) return -1;
    srt_setsockflag(s, SRTO_PBKEYLEN, &klen, sizeof klen);
#endif
    return 0;
}
#endif
