#include <stdio.h>
#include <stdlib.h>
#include "tstrans.h"

int main(void) {
    /* ABI handshake: proves the Rust staticlib linked + the constant is 9. */
    unsigned minor = tst_get_abi_version_minor();
    if (minor != (unsigned)TST_ABI_VERSION_MINOR) {
        printf("FAIL[abi]: runtime=%u header=%d\n", minor, TST_ABI_VERSION_MINOR);
        return 1;
    }
    /* Allocator smoke: tst_mux_config_new allocates via the Rust global
       allocator -> newlib memalign -> our _sbrk heap; free via tst_mux_config_free. */
    struct tst_mux_config_t *cfg = tst_mux_config_new();
    if (!cfg) { printf("FAIL[alloc]: tst_mux_config_new returned NULL\n"); return 1; }
    tst_mux_config_free(cfg);

    printf("SMOKE-OK abi=%u\n", minor);
    return 0;
}
