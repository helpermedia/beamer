//! Logging to the system log, for plugins that opt in.
//!
//! The framework reports problems through the `log` crate: entries of a
//! saved state it had to skip, events the audio thread had to drop, a
//! WebView that could not load. Nobody sees them unless a logger is
//! installed. A plugin opts in with `log_level` in its `Config.toml`
//! ([`Config::with_logging`]), and the format wrapper then calls [`init`]
//! when it starts.
//!
//! On macOS the messages go to the unified system log, under the subsystem
//! of the plugin and a category per plugin format. That works in every
//! host and also in the sandboxed process of an AUv3. Read them in
//! Console.app or with
//! `/usr/bin/log stream --level debug --predicate 'subsystem == "<subsystem>"'`.
//! On other platforms [`init`] does nothing yet.
//!
//! Every plugin binary has its own logger, so a plugin cannot get in the
//! way of the host or of other plugins. All instances of a plugin share it.
//! A plugin that installed a logger of its own before keeps it.

use std::sync::OnceLock;

use crate::config::Config;

/// How much a plugin logs. Each level includes the ones before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    /// Only errors.
    Error,
    /// Errors and warnings: what went wrong but did not stop the plugin.
    Warn,
    /// Also notes on what the plugin does.
    Info,
    /// Also details for debugging.
    Debug,
    /// Everything.
    Trace,
}

impl LogLevel {
    fn filter(self) -> log::LevelFilter {
        match self {
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Debug => log::LevelFilter::Debug,
            LogLevel::Trace => log::LevelFilter::Trace,
        }
    }
}

/// The logger of this plugin binary, once [`init`] made one.
static LOGGER: OnceLock<SystemLogger> = OnceLock::new();

/// Install the logger of the plugin, if its configuration asks for one.
///
/// For format wrappers, which call it when they start, before anything
/// logs. `format` names the plugin format ("vst3", "au") and becomes the
/// category of the messages. Only the first call does something.
pub fn init(config: &Config, format: &'static str) {
    let Some(level) = config.log_level else {
        return;
    };
    let mut created = false;
    let logger = LOGGER.get_or_init(|| {
        created = true;
        SystemLogger::new(config.log_subsystem, format)
    });
    // A plugin that installed a logger of its own keeps it.
    if created && log::set_logger(logger).is_ok() {
        log::set_max_level(level.filter());
    }
}

/// The text of a record as it goes to the system log: the level, where it
/// comes from and the message. A NUL character, which would end the text
/// early, becomes a space.
fn line(record: &log::Record<'_>) -> String {
    format!("{} {}: {}", record.level(), record.target(), record.args()).replace('\0', " ")
}

#[cfg(target_os = "macos")]
use system_log::SystemLogger;

#[cfg(target_os = "macos")]
mod system_log {
    use std::ffi::{c_char, c_void, CString};

    /// `os_log_type_t` values from `<os/log.h>`.
    const OS_LOG_TYPE_DEFAULT: u8 = 0x00;
    const OS_LOG_TYPE_DEBUG: u8 = 0x02;
    const OS_LOG_TYPE_ERROR: u8 = 0x10;

    unsafe extern "C" {
        fn os_log_create(subsystem: *const c_char, category: *const c_char) -> *mut c_void;
        fn beamer_os_log(log: *mut c_void, kind: u8, message: *const c_char);
    }

    /// A `log` logger that writes to the unified system log.
    pub(super) struct SystemLogger {
        /// The `os_log_t` of the plugin's subsystem and category. It is
        /// never released: the logger lives as long as the binary.
        log: *mut c_void,
    }

    // SAFETY: An os_log_t may be used from any thread at the same time, and
    // the pointer is never changed.
    unsafe impl Send for SystemLogger {}
    // SAFETY: See above.
    unsafe impl Sync for SystemLogger {}

    impl SystemLogger {
        pub(super) fn new(subsystem: &str, category: &str) -> Self {
            let subsystem = CString::new(subsystem.replace('\0', " ")).unwrap_or_default();
            let category = CString::new(category.replace('\0', " ")).unwrap_or_default();
            // SAFETY: Both are valid C strings, which os_log_create copies.
            let log = unsafe { os_log_create(subsystem.as_ptr(), category.as_ptr()) };
            Self { log }
        }
    }

    /// The type of the system log a level goes to. Warnings and notes go to
    /// the default type, which the system keeps on disk; debug messages are
    /// kept in memory only and show up in a live stream.
    pub(super) fn kind(level: log::Level) -> u8 {
        match level {
            log::Level::Error => OS_LOG_TYPE_ERROR,
            log::Level::Warn | log::Level::Info => OS_LOG_TYPE_DEFAULT,
            log::Level::Debug | log::Level::Trace => OS_LOG_TYPE_DEBUG,
        }
    }

    impl log::Log for SystemLogger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::max_level()
        }

        fn log(&self, record: &log::Record<'_>) {
            if !self.enabled(record.metadata()) {
                return;
            }
            let Ok(text) = CString::new(super::line(record)) else {
                return;
            };
            // SAFETY: log is the os_log_t made in new() and text a valid C
            // string for the call.
            unsafe { beamer_os_log(self.log, kind(record.level()), text.as_ptr()) };
        }

        fn flush(&self) {}
    }
}

#[cfg(not(target_os = "macos"))]
use other::SystemLogger;

#[cfg(not(target_os = "macos"))]
mod other {
    /// Without a system log to write to, the logger drops every message.
    pub(super) struct SystemLogger;

    impl SystemLogger {
        pub(super) fn new(_subsystem: &str, _category: &str) -> Self {
            Self
        }
    }

    impl log::Log for SystemLogger {
        fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
            false
        }

        fn log(&self, _record: &log::Record<'_>) {}

        fn flush(&self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_has_level_target_and_message() {
        let text = line(
            &log::Record::builder()
                .level(log::Level::Warn)
                .target("beamer_au::render")
                .args(format_args!("dropped {}\0events", 3))
                .build(),
        );
        assert_eq!(text, "WARN beamer_au::render: dropped 3 events");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_levels_go_to_the_types_of_the_system_log() {
        assert_eq!(system_log::kind(log::Level::Error), 0x10);
        assert_eq!(system_log::kind(log::Level::Warn), 0x00);
        assert_eq!(system_log::kind(log::Level::Info), 0x00);
        assert_eq!(system_log::kind(log::Level::Debug), 0x02);
        assert_eq!(system_log::kind(log::Level::Trace), 0x02);
    }

    #[test]
    fn test_level_filters() {
        assert_eq!(LogLevel::Error.filter(), log::LevelFilter::Error);
        assert_eq!(LogLevel::Warn.filter(), log::LevelFilter::Warn);
        assert_eq!(LogLevel::Trace.filter(), log::LevelFilter::Trace);
    }
}
