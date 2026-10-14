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
    /// Format: "12.0", "-60.0" (unit "dB" via `unit()`)
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

/// Strip a unit from the end of a text, in any case and with or without a
/// space before it.
fn strip_unit<'a>(s: &'a str, unit: &str) -> &'a str {
    let split = s.len().saturating_sub(unit.len());
    match (s.get(..split), s.get(split..)) {
        (Some(number), Some(end)) if end.eq_ignore_ascii_case(unit) => number.trim_end(),
        _ => s,
    }
}

/// Whether a text names minus infinity, as a user types it for silence.
fn is_minus_infinity(s: &str) -> bool {
    ["-inf", "-infinity", "-∞"]
        .iter()
        .any(|text| s.eq_ignore_ascii_case(text))
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
                    let db = if db == 0.0 { 0.0 } else { db };
                    format!("{:.prec$}", db, prec = *precision)
                }
            }

            Formatter::DecibelDirect { precision, min_db } => {
                // Value is already in dB, just format it.
                // Use strict less-than so that min_db itself displays correctly.
                // Normalize -0.0 to 0.0 to avoid displaying "-0.0".
                if value < *min_db {
                    "-inf".to_string()
                } else {
                    let value = if value == 0.0 { 0.0 } else { value };
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
                    format!("L {:.0}", value.abs() * 100.0)
                } else {
                    format!("R {:.0}", value * 100.0)
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
        // Text such as "nan" or "inf" reads as a float, but it is no value
        // a user means. Only a ratio has an infinite value ("∞:1").
        self.parse_number(s.trim()).filter(|value| {
            value.is_finite() || (*value == f64::INFINITY && matches!(self, Formatter::Ratio { .. }))
        })
    }

    /// The plain value of a text, before non-finite values are dropped.
    fn parse_number(&self, s: &str) -> Option<f64> {
        match self {
            Formatter::Float { .. } => s.parse().ok(),

            Formatter::Decibel { .. } => {
                let trimmed = strip_unit(s, "db");

                if is_minus_infinity(trimmed) {
                    return Some(0.0);
                }

                let db: f64 = trimmed.parse().ok()?;
                Some(10.0_f64.powf(db / 20.0))
            }

            Formatter::DecibelDirect { min_db, .. } => {
                // Parse dB value directly (no conversion)
                let trimmed = strip_unit(s, "db");

                if is_minus_infinity(trimmed) {
                    return Some(*min_db);
                }

                trimmed.parse().ok()
            }

            Formatter::Frequency => {
                // The unit is optional and its case does not matter. A "k"
                // multiplies by 1000, with or without "Hz" after it: `text`
                // writes "1.50k", and a host may append the unit.
                let lower = s.to_ascii_lowercase();
                let number = lower.strip_suffix("hz").unwrap_or(&lower).trim_end();

                match number.strip_suffix('k') {
                    Some(khz) => khz.trim_end().parse::<f64>().ok().map(|v| v * 1000.0),
                    None => number.parse().ok(),
                }
            }

            Formatter::Milliseconds { .. } => strip_unit(s, "ms").parse().ok(),

            Formatter::Seconds { .. } => strip_unit(s, "s").parse().ok(),

            Formatter::Percent { .. } => strip_unit(s, "%").parse::<f64>().ok().map(|v| v / 100.0),

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
                if ["∞", "∞:1", "inf:1", "infinity:1"].iter().any(|text| s.eq_ignore_ascii_case(text)) {
                    return Some(f64::INFINITY);
                }

                // Strip ":1" suffix
                let trimmed = s.trim_end_matches(":1").trim();
                trimmed.parse().ok()
            }

            Formatter::Semitones => strip_unit(s, "st").parse().ok(),

            Formatter::Boolean => match s.to_lowercase().as_str() {
                "on" | "true" | "yes" | "enabled" => Some(1.0),
                "off" | "false" | "no" | "disabled" => Some(0.0),
                // A number, as a host may type it ("1", "0.0"): on from 0.5,
                // like the text of the value.
                other => other
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .map(|value| if value > 0.5 { 1.0 } else { 0.0 }),
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

    /// Get a stable identifier for this formatter variant.
    ///
    /// Used by the webview IPC protocol so JavaScript can implement
    /// custom formatting per parameter kind.
    pub fn kind(&self) -> &'static str {
        match self {
            Formatter::Float { .. } => "float",
            Formatter::Decibel { .. } => "db",
            Formatter::DecibelDirect { .. } => "db",
            Formatter::Frequency => "hz",
            Formatter::Milliseconds { .. } => "ms",
            Formatter::Seconds { .. } => "s",
            Formatter::Percent { .. } => "percent",
            Formatter::Pan => "pan",
            Formatter::Ratio { .. } => "ratio",
            Formatter::Semitones => "semitones",
            Formatter::Boolean => "boolean",
        }
    }
}

impl Default for Formatter {
    fn default() -> Self {
        Formatter::Float { precision: 2 }
    }
}

impl Formatter {
    /// Return a new `Formatter` with updated precision.
    ///
    /// For formatter variants that have a `precision` field, this returns
    /// a new formatter with the updated precision. For variants without
    /// precision (e.g., `Pan`, `Boolean`, `Semitones`, `Frequency`), this
    /// returns `self` unchanged.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let formatter = Formatter::DecibelDirect { precision: 1, min_db: -60.0 };
    /// let high_precision = formatter.with_precision(3);
    /// // high_precision is DecibelDirect { precision: 3, min_db: -60.0 }
    ///
    /// let pan = Formatter::Pan;
    /// let same_pan = pan.with_precision(2);
    /// // same_pan is still Pan (no precision field)
    /// ```
    pub fn with_precision(self, precision: usize) -> Self {
        match self {
            Formatter::Float { .. } => Formatter::Float { precision },
            Formatter::Decibel { .. } => Formatter::Decibel { precision },
            Formatter::DecibelDirect { min_db, .. } => {
                Formatter::DecibelDirect { precision, min_db }
            }
            Formatter::Milliseconds { .. } => Formatter::Milliseconds { precision },
            Formatter::Seconds { .. } => Formatter::Seconds { precision },
            Formatter::Percent { .. } => Formatter::Percent { precision },
            Formatter::Ratio { .. } => Formatter::Ratio { precision },
            // Variants without precision return self unchanged
            Formatter::Frequency
            | Formatter::Pan
            | Formatter::Semitones
            | Formatter::Boolean => self,
        }
    }

    /// Check if this formatter variant supports precision customization.
    ///
    /// Returns `true` for variants with a `precision` field, `false` otherwise.
    pub fn supports_precision(&self) -> bool {
        matches!(
            self,
            Formatter::Float { .. }
                | Formatter::Decibel { .. }
                | Formatter::DecibelDirect { .. }
                | Formatter::Milliseconds { .. }
                | Formatter::Seconds { .. }
                | Formatter::Percent { .. }
                | Formatter::Ratio { .. }
        )
    }

    /// Get the current precision, if applicable.
    ///
    /// Returns `Some(precision)` for variants with a `precision` field,
    /// `None` otherwise.
    pub fn precision(&self) -> Option<usize> {
        match self {
            Formatter::Float { precision }
            | Formatter::Decibel { precision }
            | Formatter::DecibelDirect { precision, .. }
            | Formatter::Milliseconds { precision }
            | Formatter::Seconds { precision }
            | Formatter::Percent { precision }
            | Formatter::Ratio { precision } => Some(*precision),
            Formatter::Frequency | Formatter::Pan | Formatter::Semitones | Formatter::Boolean => {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_with_precision_float() {
        let formatter = Formatter::Float { precision: 2 };
        let updated = formatter.with_precision(4);
        assert_eq!(updated.precision(), Some(4));
        assert_eq!(updated.text(1.2345), "1.2345");
    }

    #[test]
    fn test_with_precision_decibel() {
        let formatter = Formatter::Decibel { precision: 1 };
        let updated = formatter.with_precision(2);
        assert_eq!(updated.precision(), Some(2));
        assert_eq!(updated.text(1.0), "0.00"); // 0 dB
    }

    #[test]
    fn test_with_precision_decibel_direct() {
        let formatter = Formatter::DecibelDirect {
            precision: 1,
            min_db: -60.0,
        };
        let updated = formatter.with_precision(3);
        assert_eq!(updated.precision(), Some(3));
        // Verify min_db is preserved
        if let Formatter::DecibelDirect { min_db, precision } = updated {
            assert_eq!(min_db, -60.0);
            assert_eq!(precision, 3);
        } else {
            panic!("Expected DecibelDirect variant");
        }
        assert_eq!(updated.text(-6.5), "-6.500");
    }

    #[test]
    fn test_with_precision_milliseconds() {
        let formatter = Formatter::Milliseconds { precision: 1 };
        let updated = formatter.with_precision(0);
        assert_eq!(updated.precision(), Some(0));
        assert_eq!(updated.text(10.5), "10"); // Rounded to 0 decimal places
    }

    #[test]
    fn test_with_precision_seconds() {
        let formatter = Formatter::Seconds { precision: 2 };
        let updated = formatter.with_precision(3);
        assert_eq!(updated.precision(), Some(3));
        assert_eq!(updated.text(1.5), "1.500");
    }

    #[test]
    fn test_with_precision_percent() {
        let formatter = Formatter::Percent { precision: 0 };
        let updated = formatter.with_precision(1);
        assert_eq!(updated.precision(), Some(1));
        assert_eq!(updated.text(0.755), "75.5"); // 0.755 * 100 = 75.5
    }

    #[test]
    fn test_with_precision_ratio() {
        let formatter = Formatter::Ratio { precision: 1 };
        let updated = formatter.with_precision(2);
        assert_eq!(updated.precision(), Some(2));
        assert_eq!(updated.text(4.0), "4.00:1");
    }

    #[test]
    fn test_with_precision_no_effect_on_frequency() {
        let formatter = Formatter::Frequency;
        let updated = formatter.with_precision(5);
        assert_eq!(updated, Formatter::Frequency);
        assert_eq!(updated.precision(), None);
    }

    #[test]
    fn test_frequency_parses_its_own_text() {
        let formatter = Formatter::Frequency;

        for hz in [20.0, 440.0, 1500.0, 4000.0, 20000.0] {
            let text = formatter.text(hz);
            assert_eq!(formatter.parse(&text), Some(hz), "\"{}\"", text);
        }

        // What a user types.
        assert_eq!(formatter.parse("4k"), Some(4000.0));
        assert_eq!(formatter.parse("4 K"), Some(4000.0));
        assert_eq!(formatter.parse("1.5 kHz"), Some(1500.0));
        assert_eq!(formatter.parse("440 Hz"), Some(440.0));
        assert_eq!(formatter.parse("2e3"), Some(2000.0));

        // The text with the unit after it, and the unit in any case.
        assert_eq!(formatter.parse("1.50k Hz"), Some(1500.0));
        assert_eq!(formatter.parse("1.5 Khz"), Some(1500.0));
        assert_eq!(formatter.parse("1.5KHZ"), Some(1500.0));
        assert_eq!(formatter.parse("440HZ"), Some(440.0));

        for text in ["", "k", "Hz", "kHz", "abc", "1.5 kHz Hz", "4 kk"] {
            assert_eq!(formatter.parse(text), None, "\"{}\"", text);
        }
    }

    #[test]
    fn test_units_parse_in_any_case() {
        let db = Formatter::Decibel { precision: 1 };
        assert_eq!(db.parse("0 dB"), Some(1.0));
        assert_eq!(db.parse("0db"), Some(1.0));
        assert_eq!(db.parse(" 0 DB "), Some(1.0));
        assert_eq!(db.parse("-INF"), Some(0.0));
        assert_eq!(db.parse("-Infinity dB"), Some(0.0));

        let direct = Formatter::DecibelDirect { precision: 1, min_db: -60.0 };
        assert_eq!(direct.parse("-6 db"), Some(-6.0));
        assert_eq!(direct.parse("-inf DB"), Some(-60.0));

        let ms = Formatter::Milliseconds { precision: 1 };
        assert_eq!(ms.parse("10 MS"), Some(10.0));
        assert_eq!(ms.parse("10Ms"), Some(10.0));

        let seconds = Formatter::Seconds { precision: 2 };
        assert_eq!(seconds.parse("2.5 S"), Some(2.5));

        assert_eq!(Formatter::Semitones.parse("7 ST"), Some(7.0));
        assert_eq!(Formatter::Semitones.parse("-12st"), Some(-12.0));
        assert_eq!(Formatter::Percent { precision: 0 }.parse("50 %"), Some(0.5));

        // A unit is no number.
        assert_eq!(ms.parse("ms"), None);
        assert_eq!(db.parse("dB"), None);
    }

    #[test]
    fn test_values_that_are_no_number_do_not_parse() {
        let formatters = [
            Formatter::Float { precision: 2 },
            Formatter::Decibel { precision: 1 },
            Formatter::DecibelDirect { precision: 1, min_db: -60.0 },
            Formatter::Frequency,
            Formatter::Milliseconds { precision: 1 },
            Formatter::Seconds { precision: 2 },
            Formatter::Percent { precision: 0 },
            Formatter::Pan,
            Formatter::Ratio { precision: 1 },
            Formatter::Semitones,
            Formatter::Boolean,
        ];
        for formatter in &formatters {
            for text in ["nan", "NaN", "-nan", "nan dB", "-inf ms"] {
                assert_eq!(formatter.parse(text), None, "{:?} \"{}\"", formatter, text);
            }
            // Infinity is a value of a ratio only.
            if !matches!(formatter, Formatter::Ratio { .. }) {
                for text in ["inf", "infinity", "+inf"] {
                    assert_eq!(formatter.parse(text), None, "{:?} \"{}\"", formatter, text);
                }
            }
        }

        // The one infinite value a formatter writes itself.
        let ratio = Formatter::Ratio { precision: 1 };
        assert_eq!(ratio.parse("∞:1"), Some(f64::INFINITY));
        assert_eq!(ratio.parse("∞"), Some(f64::INFINITY));
        assert_eq!(ratio.parse("INF:1"), Some(f64::INFINITY));
        assert_eq!(ratio.parse("inf"), Some(f64::INFINITY));
        assert_eq!(ratio.parse("-inf"), None);
        assert_eq!(ratio.parse("4:1"), Some(4.0));
    }

    #[test]
    fn test_boolean_parses_a_number() {
        let boolean = Formatter::Boolean;
        assert_eq!(boolean.parse("On"), Some(1.0));
        assert_eq!(boolean.parse("off"), Some(0.0));
        assert_eq!(boolean.parse("1"), Some(1.0));
        assert_eq!(boolean.parse("1.0"), Some(1.0));
        assert_eq!(boolean.parse(" 0.000 "), Some(0.0));
        assert_eq!(boolean.parse("0.5"), Some(0.0));
        assert_eq!(boolean.parse("maybe"), None);
    }

    #[test]
    fn test_with_precision_no_effect_on_pan() {
        let formatter = Formatter::Pan;
        let updated = formatter.with_precision(3);
        assert_eq!(updated, Formatter::Pan);
        assert_eq!(updated.precision(), None);
    }

    #[test]
    fn test_with_precision_no_effect_on_semitones() {
        let formatter = Formatter::Semitones;
        let updated = formatter.with_precision(2);
        assert_eq!(updated, Formatter::Semitones);
        assert_eq!(updated.precision(), None);
    }

    #[test]
    fn test_with_precision_no_effect_on_boolean() {
        let formatter = Formatter::Boolean;
        let updated = formatter.with_precision(1);
        assert_eq!(updated, Formatter::Boolean);
        assert_eq!(updated.precision(), None);
    }

    #[test]
    fn test_supports_precision() {
        assert!(Formatter::Float { precision: 2 }.supports_precision());
        assert!(Formatter::Decibel { precision: 1 }.supports_precision());
        assert!(Formatter::DecibelDirect {
            precision: 1,
            min_db: -60.0
        }
        .supports_precision());
        assert!(Formatter::Milliseconds { precision: 1 }.supports_precision());
        assert!(Formatter::Seconds { precision: 2 }.supports_precision());
        assert!(Formatter::Percent { precision: 0 }.supports_precision());
        assert!(Formatter::Ratio { precision: 1 }.supports_precision());

        assert!(!Formatter::Frequency.supports_precision());
        assert!(!Formatter::Pan.supports_precision());
        assert!(!Formatter::Semitones.supports_precision());
        assert!(!Formatter::Boolean.supports_precision());
    }

    #[test]
    fn test_precision_getter() {
        assert_eq!(Formatter::Float { precision: 3 }.precision(), Some(3));
        assert_eq!(Formatter::Decibel { precision: 2 }.precision(), Some(2));
        assert_eq!(
            Formatter::DecibelDirect {
                precision: 1,
                min_db: -60.0
            }
            .precision(),
            Some(1)
        );
        assert_eq!(Formatter::Frequency.precision(), None);
        assert_eq!(Formatter::Pan.precision(), None);
    }
}
