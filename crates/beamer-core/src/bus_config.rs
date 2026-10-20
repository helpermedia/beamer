//! Cached bus configuration for format wrappers.
//!
//! This module provides types for caching bus configuration after it has been
//! extracted from the plugin or host. This avoids repeated queries and provides
//! fast access during audio processing.

use crate::active_buses::ActiveBuses;
use crate::plugin::{BusInfo, BusType, Model};
use crate::types::{MAX_BUSES, MAX_CHANNELS};

/// Lightweight bus information for caching.
///
/// Contains only the data needed for buffer allocation and validation.
/// This is distinct from [`BusInfo`] which contains full metadata (name, etc.)
/// for host queries.
#[derive(Clone, Copy, Debug, Default)]
pub struct CachedBusInfo {
    /// Number of channels in this bus.
    pub channel_count: usize,
    /// Bus type (main or auxiliary).
    pub bus_type: BusType,
}

impl CachedBusInfo {
    /// Create a new cached bus info.
    pub const fn new(channel_count: usize, bus_type: BusType) -> Self {
        Self {
            channel_count,
            bus_type,
        }
    }

    /// Create from a full BusInfo (drops name and is_default_active).
    pub fn from_bus_info(info: &BusInfo) -> Self {
        Self {
            channel_count: info.channel_count as usize,
            bus_type: info.bus_type,
        }
    }
}

/// Cached bus configuration from plugin or host.
///
/// Stores bus and channel information for fast access during audio processing.
/// This provides a common representation used by both VST3 and AU wrappers.
#[derive(Clone, Debug)]
pub struct CachedBusConfig {
    /// Number of input buses.
    pub input_bus_count: usize,
    /// Number of output buses.
    pub output_bus_count: usize,
    /// Input bus information.
    pub input_buses: Vec<CachedBusInfo>,
    /// Output bus information.
    pub output_buses: Vec<CachedBusInfo>,
}

impl CachedBusConfig {
    /// Create a new cached bus configuration.
    ///
    /// # Panics
    ///
    /// Panics if bus counts exceed MAX_BUSES.
    pub fn new(input_buses: Vec<CachedBusInfo>, output_buses: Vec<CachedBusInfo>) -> Self {
        assert!(
            input_buses.len() <= MAX_BUSES,
            "Input bus count {} exceeds MAX_BUSES ({})",
            input_buses.len(),
            MAX_BUSES
        );
        assert!(
            output_buses.len() <= MAX_BUSES,
            "Output bus count {} exceeds MAX_BUSES ({})",
            output_buses.len(),
            MAX_BUSES
        );

        Self {
            input_bus_count: input_buses.len(),
            output_bus_count: output_buses.len(),
            input_buses,
            output_buses,
        }
    }

    /// Create from the bus configuration a model declares.
    pub fn from_model<P: Model>(model: &P) -> Self {
        let input_bus_count = model.input_bus_count();
        let output_bus_count = model.output_bus_count();

        let input_buses: Vec<CachedBusInfo> = (0..input_bus_count)
            .filter_map(|i| model.input_bus_info(i).map(|b| CachedBusInfo::from_bus_info(&b)))
            .collect();

        let output_buses: Vec<CachedBusInfo> = (0..output_bus_count)
            .filter_map(|i| model.output_bus_info(i).map(|b| CachedBusInfo::from_bus_info(&b)))
            .collect();

        Self {
            input_bus_count,
            output_bus_count,
            input_buses,
            output_buses,
        }
    }

    /// Get information about an input bus.
    ///
    /// Returns `None` if the bus index is out of bounds.
    pub fn input_bus_info(&self, bus: usize) -> Option<&CachedBusInfo> {
        self.input_buses.get(bus)
    }

    /// Get information about an output bus.
    ///
    /// Returns `None` if the bus index is out of bounds.
    pub fn output_bus_info(&self, bus: usize) -> Option<&CachedBusInfo> {
        self.output_buses.get(bus)
    }

    /// Get the total number of input channels across all buses.
    pub fn total_input_channels(&self) -> usize {
        self.input_buses.iter().map(|b| b.channel_count).sum()
    }

    /// Get the total number of output channels across all buses.
    pub fn total_output_channels(&self) -> usize {
        self.output_buses.iter().map(|b| b.channel_count).sum()
    }

    /// The buses of this configuration, each on with its channel count
    /// (off with 0 channels).
    ///
    /// The wrappers prepare a plugin with these until they negotiate the
    /// buses with the host.
    pub fn to_active_buses(&self) -> ActiveBuses {
        let mut buses = ActiveBuses::all_off(self.input_bus_count, self.output_bus_count);
        for (bus, info) in self.input_buses.iter().enumerate() {
            let _ = buses.set_input(bus, Some(info.channel_count as u32));
        }
        for (bus, info) in self.output_buses.iter().enumerate() {
            let _ = buses.set_output(bus, Some(info.channel_count as u32));
        }
        buses
    }

    /// Validate that this configuration doesn't exceed system limits.
    ///
    /// Checks that:
    /// - Bus counts are within MAX_BUSES
    /// - Channel counts per bus are within MAX_CHANNELS
    ///
    /// Returns `Ok(())` if valid, or `Err` with a descriptive message.
    pub fn validate(&self) -> Result<(), String> {
        // Validate bus counts
        if self.input_bus_count > MAX_BUSES {
            return Err(format!(
                "Plugin declares {} input buses, but MAX_BUSES is {}",
                self.input_bus_count, MAX_BUSES
            ));
        }
        if self.output_bus_count > MAX_BUSES {
            return Err(format!(
                "Plugin declares {} output buses, but MAX_BUSES is {}",
                self.output_bus_count, MAX_BUSES
            ));
        }

        // Validate channel counts for each input bus
        for (i, bus) in self.input_buses.iter().enumerate() {
            if bus.channel_count > MAX_CHANNELS {
                return Err(format!(
                    "Input bus {} declares {} channels, but MAX_CHANNELS is {}",
                    i, bus.channel_count, MAX_CHANNELS
                ));
            }
        }

        // Validate channel counts for each output bus
        for (i, bus) in self.output_buses.iter().enumerate() {
            if bus.channel_count > MAX_CHANNELS {
                return Err(format!(
                    "Output bus {} declares {} channels, but MAX_CHANNELS is {}",
                    i, bus.channel_count, MAX_CHANNELS
                ));
            }
        }

        Ok(())
    }
}

impl Default for CachedBusConfig {
    /// Create a default stereo configuration (2in/2out, main bus only).
    fn default() -> Self {
        Self::new(
            vec![CachedBusInfo::new(2, BusType::Main)],
            vec![CachedBusInfo::new(2, BusType::Main)],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = CachedBusConfig::default();
        assert_eq!(config.input_bus_count, 1);
        assert_eq!(config.output_bus_count, 1);
        assert_eq!(config.total_input_channels(), 2);
        assert_eq!(config.total_output_channels(), 2);
    }

    #[test]
    fn test_validate_success() {
        let config = CachedBusConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_to_active_buses() {
        let config = CachedBusConfig::new(
            vec![
                CachedBusInfo::new(2, BusType::Main),
                CachedBusInfo::new(1, BusType::Aux),
                CachedBusInfo::new(0, BusType::Aux),
            ],
            vec![CachedBusInfo::new(6, BusType::Main)],
        );
        let buses = config.to_active_buses();
        assert_eq!(buses.input_bus_count(), 3);
        assert_eq!(buses.output_bus_count(), 1);
        assert_eq!(buses.input(0), Some(2));
        assert_eq!(buses.input(1), Some(1));
        assert!(!buses.is_input_on(2));
        assert_eq!(buses.output(0), Some(6));

        let empty = CachedBusConfig::new(Vec::new(), Vec::new()).to_active_buses();
        assert_eq!(empty.input_bus_count(), 0);
        assert_eq!(empty.output(0), None);
    }

    #[test]
    fn test_cached_bus_info_from_bus_info() {
        let bus_info = BusInfo {
            name: "Test Bus",
            bus_type: BusType::Aux,
            channel_count: 4,
            is_default_active: true,
        };
        let cached = CachedBusInfo::from_bus_info(&bus_info);
        assert_eq!(cached.channel_count, 4);
        assert_eq!(cached.bus_type, BusType::Aux);
    }

    #[test]
    fn test_empty_config() {
        let config = CachedBusConfig::new(vec![], vec![]);
        assert_eq!(config.input_bus_count, 0);
        assert_eq!(config.output_bus_count, 0);
        assert_eq!(config.total_input_channels(), 0);
        assert_eq!(config.total_output_channels(), 0);
        assert!(config.validate().is_ok());
    }
}
