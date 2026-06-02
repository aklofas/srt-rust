// S2 — boot smoke: cross-compiled libsrt initializes on the FreeRTOS+lwIP
// substrate. srt_startup spawns the SRT:GC garbage-collector thread as a
// FreeRTOS-Plus-POSIX pthread; srt_create_socket allocates a CUDTSocket;
// srt_cleanup joins the GC thread. No bind/connect — pure runtime init/teardown
// (the data plane is S3). lwIP is linked so libsrt's socket-call symbols
// resolve (the R4 deliverable), but no datagram flows.
#include <cstdio>
#include <cstdint>
// srt.h pulls C++ stdlib headers and carries its own `extern "C"` guards, so it
// must be included at C++ linkage (NOT inside an extern "C" block).
#include <srt/srt.h>
extern "C" {
#include "FreeRTOS.h"
#include "task.h"
}
// _exit (semihosting, from rdimon.specs) is declared via srt.h's <unistd.h>.

static void boot_task(void*) {
    int fail = 0;
    const char* where = "";

    if (srt_startup() < 0)               { fail = 1; where = "startup"; }
    SRTSOCKET s = SRT_INVALID_SOCK;
    if (!fail) {
        s = srt_create_socket();
        if (s == SRT_INVALID_SOCK)       { fail = 1; where = "create_socket"; }
    }
    // Behavioral check: a freshly created socket must report SRTS_INIT. This
    // proves libsrt's runtime tracks real per-socket state (the gate isn't
    // vacuous), and is the RED-proof hook: flip SRTS_INIT to another state and
    // the gate must FAIL. (Note: libsrt lazily constructs its global singleton
    // at static init, so srt_create_socket succeeds even before srt_startup —
    // hence a state check, not a create-before-startup check, is the real RED.)
    if (!fail && srt_getsockstate(s) != SRTS_INIT) { fail = 1; where = "sockstate"; }
    if (!fail && srt_close(s) == SRT_ERROR)   { fail = 1; where = "close"; }
    if (!fail && srt_cleanup() == SRT_ERROR)  { fail = 1; where = "cleanup"; }

    if (fail) printf("FAIL[s2_libsrt]: %s: %s\n", where, srt_getlasterror_str());
    else      printf("PASS: s2_libsrt (startup+socket+cleanup)\n");
    fflush(stdout);
    _exit(fail ? 1 : 0);
}

int main() {
    // Generous stack: srt_startup + the GC thread exercise the C++ unwinder
    // (S0 finding: libsrt threads need >=4 KiB; the bootstrap task more).
    xTaskCreate(boot_task, "boot", 4096, nullptr, 2, nullptr);
    vTaskStartScheduler();
    for (;;) {}
}

extern "C" void vApplicationMallocFailedHook(void) { printf("FAIL[malloc]\n"); _exit(1); }
extern "C" void vApplicationStackOverflowHook(TaskHandle_t, char*) { printf("FAIL[stack]\n"); _exit(1); }
