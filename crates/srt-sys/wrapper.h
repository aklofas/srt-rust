/* wrapper.h — single header consumed by bindgen for libsrt's public API.
 *
 * Including srt.h alone is sufficient; transitive headers (logging_api.h,
 * platform_sys.h, etc.) are pulled in by srt.h itself. access_control.h is
 * NOT pulled in transitively, so we include it explicitly to expose the
 * SRT_REJX_* extension reject codes (SRT_REJC_PREDEFINED range, 1000+).
 */

#include <srt/srt.h>
#include <srt/access_control.h>
