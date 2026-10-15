// Bridge from Rust to the macOS unified system log. os_log_with_type is a
// macro that needs a format string known at compile time, so the message
// is passed as one public string argument.

#include <os/log.h>
#include <stdint.h>

void beamer_os_log(os_log_t log, uint8_t type, const char *message) {
    os_log_with_type(log, (os_log_type_t)type, "%{public}s", message);
}
