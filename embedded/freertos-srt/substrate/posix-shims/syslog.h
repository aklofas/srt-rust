/* Bare-metal newlib ships no <syslog.h>, but libsrt's logging_api.h includes it
 * unconditionally just for the LOG_* level constants (it says so at L33). These
 * values mirror libsrt's own common/win/syslog_defs.h exactly. No syslog()
 * function is provided — libsrt never calls it (logging is OFF here anyway). */
#ifndef FREERTOS_SRT_SHIM_SYSLOG_H
#define FREERTOS_SRT_SHIM_SYSLOG_H

#define LOG_EMERG       0
#define LOG_ALERT       1
#define LOG_CRIT        2
#define LOG_ERR         3
#define LOG_WARNING     4
#define LOG_NOTICE      5
#define LOG_INFO        6
#define LOG_DEBUG       7

#endif
