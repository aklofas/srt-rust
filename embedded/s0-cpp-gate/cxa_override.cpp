// Per-task C++ exception state for the stock single-threaded bare-metal
// libstdc++. The prebuilt libsupc++ __cxa_get_globals returns ONE GLOBAL
// __cxa_eh_globals (no gthreads) -> concurrent FreeRTOS tasks throwing would
// corrupt each other's propagation. Override both __cxa_get_globals and _fast
// with FreeRTOS-TLS-backed per-task versions. Our strong defs (object linked
// before libsupc++) resolve the symbols, so libsupc++'s eh_globals.o is never
// pulled in. BOTH must be overridden together (else eh_globals.o is pulled for
// the other -> duplicate definition of ours).
//
// __cxa_eh_globals is NOT in the public <cxxabi.h>; its layout is fixed by the
// Itanium C++ ABI. We declare the minimal struct and allocate/zero it; the
// unwinder writes its fields into our block.
#include <cstdlib>
#include <cstring>
extern "C" {
#include "FreeRTOS.h"
#include "task.h"
}

extern "C" {

// ARM EABI layout: the ARM EH unwinder (this is an arm-none-eabi build) adds a
// THIRD field after uncaughtExceptions. It's load-bearing: eh_arm.o's
// __cxa_begin_cleanup/__cxa_end_cleanup do ldr/str at [r0,#8] right after
// __cxa_get_globals, and libsupc++'s static eh_globals BSS object is 12 bytes.
// Omitting it makes malloc(sizeof) 8 bytes and the unwinder's offset-8 write a
// heap overflow into the next heap_4 block's metadata.
struct __cxa_eh_globals {
    void*        caughtExceptions;
    unsigned int uncaughtExceptions;
    void*        propagatingExceptions;   // ARM EABI only — at offset 8
};

static const BaseType_t kEhTls = 0;   // configNUM_THREAD_LOCAL_STORAGE_POINTERS >= 1

__cxa_eh_globals* __cxa_get_globals(void) noexcept {
    void* p = pvTaskGetThreadLocalStoragePointer(nullptr, kEhTls);
    if (p == nullptr) {
        p = std::malloc(sizeof(__cxa_eh_globals));
        if (p == nullptr) { for(;;); }   // trap: EH state alloc must not fail
                                         // (libc malloc, so no MallocFailedHook)
        std::memset(p, 0, sizeof(__cxa_eh_globals));
        vTaskSetThreadLocalStoragePointer(nullptr, kEhTls, p);
    }
    return static_cast<__cxa_eh_globals*>(p);
}

// The unwinder only calls _fast after a throw has already run __cxa_get_globals
// in this task, so the TLS slot is populated. Return the SAME per-task pointer.
__cxa_eh_globals* __cxa_get_globals_fast(void) noexcept {
    return static_cast<__cxa_eh_globals*>(
        pvTaskGetThreadLocalStoragePointer(nullptr, kEhTls));
}

}  // extern "C"
