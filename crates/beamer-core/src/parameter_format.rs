//! Parameter value formatting and parsing.
//!
//! This module provides the [`Formatter`] enum for converting between
//! plain parameter values and display strings. Each formatter variant
//! handles a specific unit type (dB, Hz, ms, etc.) with appropriate
//! formatting and parsing logic.
//!
//! # Design
//!
//! The formatter separates value formatting from unit strings:
//! - `text()` returns the bare value without units (e.g., "440", "-6.0")
//! - `unit()` returns the unit string (e.g., "Hz", "dB")
//! - The host/UI combines them for display (e.g., "440 Hz", "-6.0 dB")
//!
//! This separation allows proper VST3/AU parameter info where the units
//! field is separate from the formatted value string.
//!
//! # Example
//!
//! ```ignore
//! use beamer_core::parameter_format::Formatter;
//!
//! let db_formatter = Formatter::Decibel { precision: 1 };
//! assert_eq!(db_formatter.text(1.0), "0.0");   // Value only
//! assert_eq!(db_formatter.unit(), "dB");       // Unit separately
//!
//! let hz_formatter = Formatter::Frequency;
//! assert_eq!(hz_formatter.text(440.0), "440");
//! assert_eq!(hz_formatter.text(1500.0), "1.50k");  // Auto-scaled with SI prefix
//! assert_eq!(hz_formatter.unit(), "Hz");
//! ```

/// Parameter value formatter.
///
/// Defines how plain parameter values are converted to display strings
/// and parsed back from user input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Formatter {
    /// Generic float with configurable precision (e.g., "1.23").
    Float {
        /// Number of decimal places.
        precision: usize,
    },

    /// Decibel formatter for gain/level parameters.
    ///
    /// Input is linear amplitude (0.0 = silence, 1.0 = unity).
    /// Format: "-12.0", "-inf" (unit "dB" via `unit()`)
    Decibel {
        /// Number of decimal places.
        precision: usize,
    },

    /// Direct decibel formatter where input is already in dB.
    ///
    /// Used by `FloatParameter::db()` where the plain value is stored as dB.
    /// Format: "+12.0", "-60.0" (unit "dB" via `unit()`)
    DecibelDirect {
        /// Number of decimal places.
        precision: usize,
        /// Minimum dB value (below this shows "-inf")
        min_db: f64,
    },

    /// Frequency formatter with automatic Hz/kHz scaling.
    ///
    /// Format: "440", "1.50k" (unit "Hz" via `unit()`)
    Frequency,

    /// Milliseconds formatter.
    ///
    /// Format: "10.0" (unit "ms" via `unit()`)
    Milliseconds {
        /// Number of decimal places.
        precision: usize,
    },

    /// Seconds formatter.
    ///
    /// Format: "1.50" (unit "s" via `unit()`)
    Seconds {
        /// Number of decimal places.
        precision: usize,
    },

    /// Percentage formatter.
    ///
    /// Input is 0.0-1.0, display is 0-100.
    /// Format: "75" (unit "%" via `unit()`)
    Percent {
        /// Number of decimal places.
        precision: usize,
    },

    /// Pan formatter for stereo position.
    ///
    /// Input is -1.0 (left) to +1.0 (right).
    /// Display: "L50", "C", "R50"
    Pan,

    /// Ratio formatter for compressors.
    ///
    /// Display: "4.0:1", "∞:1"
    Ratio {
        /// Number of decimal places.
        precision: usize,
    },

    /// Semitones formatter for pitch shifting.
    ///
    /// Format: "+12", "-7", "0" (unit "st" via `unit()`)
    Semitones,

    /// Boolean formatter.
    ///
    /// Display: "On", "Off"
    Boolean,
}

impl Formatter {
    /// Convert a plain value to a display string (without unit).
    ///
    /// The interpretation of `value` depends on the formatter variant:
    /// - `Decibel`: linear amplitude (1.0 = 0 dB)
    /// - `Frequency`: Hz
    /// - `Milliseconds`: ms
    /// - `Seconds`: s
    /// - `Percent`: 0.0-1.0 (displayed as 0-100)
    /// - `Pan`: -1.0 to +1.0
    /// - `Ratio`: ratio value (4.0 = "4:1")
    /// - `Semitones`: integer semitones
    /// - `Boolean`: >0.5 = On, <=0.5 = Off
    pub fn text(&self, value: f64) -> String {
        match self {
            Formatter::Float { precision } => {
                format!("{:.prec$}", value, prec = *precision)
            }

            Formatter::Decibel { precision } => {
                if value < 1e-10 {
                    "-inf".to_string()
                } else {
                    let db = 20.0 * value.log10();
                    if db >= 0.0 {
                        format!("+{:.prec$}", db, prec = *precision)
                    } else {
                        format!("{:.prec$}", db, prec = *precision)
                    }
                }
            }

            Formatter::DecibelDirect { precision, min_db } => {
                // Value is already in dB, just format it
                // Use strict less-than so that min_db itself displays correctly
                if value < *min_db {
                    "-inf".to_string()
                } else if value >= 0.0 {
                    format!("+{:.prec$}", value, prec = *precision)
                } else {
                    format!("{:.prec$}", value, prec = *precision)
                }
            }

            Formatter::Frequency => {
                if value >= 1000.0 {
                    format!("{:.2}k", value / 1000.0)
                } else if value >= 100.0 {
                    format!("{:.0}", value)
                } else {
                    format!("{:.1}", value)
                }
            }

            Formatter::Milliseconds { precision } => {
                format!("{:.prec$}", value, prec = *precision)
            }

            Formatter::Seconds { precision } => {
                format!("{:.prec$}", value, prec = *precision)
            }

            Formatter::Percent { precision } => {
                format!("{:.prec$}", value * 100.0, prec = *precision)
            }

            Formatter::Pan => {
                if value.abs() < 0.005 {
                    "C".to_string()
                } else if value < 0.0 {
                    format!("L{:.0}", value.abs() * 100.0)
                } else {
                    format!("R{:.0}", value * 100.0)
                }
            }

            Formatter::Ratio { precision } => {
                if value > 100.0 {
                    "∞:1".to_string()
                } else {
                    format!("{:.prec$}:1", value, prec = *precision)
                }
            }

            Formatter::Semitones => {
                let st = value.round() as i64;
                if st > 0 {
                    format!("+{}", st)
                } else {
                    format!("{}", st)
                }
            }

            Formatter::Boolean => {
                if value > 0.5 {
                    "On".to_string()
                } else {
                    "Off".to_string()
                }
            }
        }
    }

    /// Parse a display string to a plain value.
    ///
    /// Returns `None` if the string cannot be parsed.
    /// Accepts various formats with or without units.
    pub fn parse(&self, s: &str) -> Option<f64> {
        let s = s.trim();

        match self {
            Formatter::Float { .. } => s.parse().ok(),

            Formatter::Decibel { .. } => {
                let trimmed = s
                    .trim_end_matches(" dB")
                    .trim_end_matches("dB")
                    .trim();

                if trimmed.eq_ignore_ascii_case("-inf")
                    || trimmed.eq_ignore_ascii_case("-∞")
                    || trimmed == "-infinity"
                {
                    return Some(0.0);
                }

                let db: f64 = trimmed.parse().ok()?;
                Some(10.0_f64.powf(db / 20.0))
            }

            Formatter::DecibelDirect { min_db, .. } => {
                // Parse dB value directly (no conversion)
                let trimmed = s
                    .trim_end_matches(" dB")
                    .trim_end_matches("dB")
                    .trim();

                if trimmed.eq_ignore_ascii_case("-inf")
                    || trimmed.eq_ignore_ascii_case("-∞")
                    || trimmed == "-infinity"
                {
                    return Some(*min_db);
                }

                trimmed.parse().ok()
            }

            Formatter::Frequency => {
                // Try kHz first
                if let Some(khz_str) = s
                    .strip_suffix(" kHz")
                    .or_else(|| s.strip_suffix("kHz"))
                    .or_else(|| s.strip_suffix(" khz"))
                    .or_else(|| s.strip_suffix("khz"))
                {
                    return khz_str.trim().parse::<f64>().ok().map(|v| v * 1000.0);
                }

                // Then Hz
                let hz_str = s
                    .trim_end_matches(" Hz")
                    .trim_end_matches("Hz")
                    .trim_end_matches(" hz")
                    .trim_end_matches("hz")
                    .trim();

                hz_str.parse().ok()
            }

            Formatter::Milliseconds { .. } => {
                let trimmed = s
                    .strip_suffix(" ms")
                    .or_else(|| s.strip_suffix("ms"))
                    .unwrap_or(s)
                    .trim();
                trimmed.parse().ok()
            }

            Formatter::Seconds { .. } => {
                let trimmed = s
                    .strip_suffix(" s")
                    .or_else(|| s.strip_suffix("s"))
                    .unwrap_or(s)
                    .trim();
                trimmed.parse().ok()
            }

            Formatter::Percent { .. } => {
                let trimmed = s.trim_end_matches('%').trim();
                trimmed.parse::<f64>().ok().map(|v| v / 100.0)
            }

            Formatter::Pan => {
                let s_upper = s.to_uppercase();
                if s_upper == "C" || s_upper == "CENTER" || s_upper == "0" {
                    return Some(0.0);
                }

                if let Some(left) = s_upper.strip_prefix('L') {
                    return left.trim().parse::<f64>().ok().map(|v| -v / 100.0);
                }

                if let Some(right) = s_upper.strip_prefix('R') {
                    return right.trim().parse::<f64>().ok().map(|v| v / 100.0);
                }

                // Try parsing as raw number (-100 to +100 or -1 to +1)
                if let Ok(v) = s.parse::<f64>() {
                    if v.abs() > 1.0 {
                        return Some(v / 100.0); // Assume -100 to +100
                    }
                    return Some(v); // Assume -1 to +1
                }

                None
            }

            Formatter::Ratio { .. } => {
                // Handle infinity
                if s == "∞:1" || s == "inf:1" || s.eq_ignore_ascii_case("infinity:1") {
                    return Some(f64::INFINITY);
                }

                // Strip ":1" suffix
                let trimmed = s.trim_end_matches(":1").trim();
                trimmed.parse().ok()
            }

            Formatter::Semitones => {
                let trimmed = s.trim_end_matches(" st").trim_end_matches("st").trim();
                trimmed.parse().ok()
            }

            Formatter::Boolean => match s.to_lowercase().as_str() {
                "on" | "true" | "yes" | "1" | "enabled" => Some(1.0),
                "off" | "false" | "no" | "0" | "disabled" => Some(0.0),
                _ => None,
            },
        }
    }

    /// Get the unit string for this formatter.
    pub fn unit(&self) -> &'static str {
        match self {
            Formatter::Float { .. } => "",
            Formatter::Decibel { .. } => "dB",
            Formatter::DecibelDirect { .. } => "dB",
            Formatter::Frequency => "Hz",
            Formatter::Milliseconds { .. } => "ms",
            Formatter::Seconds { .. } => "s",
            Formatter::Percent { .. } => "%",
            Formatter::Pan => "",
            Formatter::Ratio { .. } => "",
            Formatter::Semitones => "st",
            Formatter::Boolean => "",
        }
    }
}

impl Default for Formatter {
    fn default() -> Self {
        Formatter::Float { precision: 2 }
    }
}
