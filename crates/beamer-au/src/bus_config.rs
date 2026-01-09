//! Bus configuration caching for Audio Unit.
//!
//! This module provides [`CachedBusConfig`], which stores the AU's bus and channel
//! configuration after it's been queried from the host. This mirrors the VST3
//! implementation's approach and enables dynamic multi-channel support.
//!
//! # Why Caching?
//!
//! Audio Unit bus configuration is queried via Objective-C messaging, which is:
//! - Relatively expensive (msg_send calls)
//! - Only needs to happen once during `allocateRenderResources`
//! - Required for multiple operations (buffer allocation, validation, processing)
//!
//! By caching the configuration, we avoid redundant queries and enable O(1) access
//! to channel counts and bus information.

use beamer_core::BusLayout;

/// Maximum number of buses supported by beamer.
///
/// Matches the limit in beamer_core (VST3 implementation).
pub const MAX_BUSES: usize = beamer_core::MAX_BUSES;

/// Maximum channels per bus supported by beamer.
///
/// Matches the limit in beamer_core (VST3 implementation).
pub const MAX_CHANNELS: usize = beamer_core::MAX_CHANNELS;

/// Cached bus configuration from Audio Unit.
///
/// Stores the bus and channel information extracted from AU's bus arrays
/// during `allocateRenderResources`. This provides fast access to configuration
/// without repeated Objective-C messaging.
///
/// # Mirrors VST3 Pattern
///
/// This struct is analogous to `CachedBusConfig` in `beamer-vst3`, ensuring
/// consistent behavior across plugin formats.
#[derive(Clone, Debug)]
pub struct CachedBusConfig {
    /// Number of input buses
    pub input_bus_count: usize,
    /// Number of output buses
    pub output_bus_count: usize,
    /// Input bus information (channel counts, etc.)
    pub input_buses: Vec<BusInfo>,
    /// Output bus information (channel counts, etc.)
    pub output_buses: Vec<BusInfo>,
}

/// Information about a single audio bus.
///
/// Simplified version focused on AU needs (channel count primarily).
#[derive(Clone, Debug)]
pub struct BusInfo {
    /// Number of channels in this bus
    pub channel_count: usize,
    /// Bus type (main or auxiliary)
    pub bus_type: BusType,
}

/// Bus type classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusType {
    /// Main bus (bus 0, always present)
    Main,
    /// Auxiliary bus (sidechain, additional outputs, etc.)
    Auxiliary,
}

impl CachedBusConfig {
    /// Create a new cached bus configuration.
    ///
    /// # Arguments
    ///
    /// * `input_buses` - Input bus information
    /// * `output_buses` - Output bus information
    ///
    /// # Panics
    ///
    /// Panics if bus counts exceed MAX_BUSES.
    pub fn new(input_buses: Vec<BusInfo>, output_buses: Vec<BusInfo>) -> Self {
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

    /// Get information about an input bus.
    ///
    /// Returns `None` if the bus index is out of bounds.
    pub fn input_bus_info(&self, bus: usize) -> Option<&BusInfo> {
        self.input_buses.get(bus)
    }

    /// Get information about an output bus.
    ///
    /// Returns `None` if the bus index is out of bounds.
    pub fn output_bus_info(&self, bus: usize) -> Option<&BusInfo> {
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

    /// Convert to beamer_core's BusLayout for plugin preparation.
    ///
    /// This enables passing the AU's bus configuration to the plugin's
    /// `prepare()` method via `FullAudioSetup`.
    pub fn to_bus_layout(&self) -> BusLayout {
        let main_in_channels = self
            .input_bus_info(0)
            .map(|b| b.channel_count as u32)
            .unwrap_or(0);
        let main_out_channels = self
            .output_bus_info(0)
            .map(|b| b.channel_count as u32)
            .unwrap_or(0);

        // Auxiliary bus count is total buses minus the main bus (if present)
        let aux_in_count = self.input_bus_count.saturating_sub(1);
        let aux_out_count = self.output_bus_count.saturating_sub(1);

        BusLayout {
            main_input_channels: main_in_channels,
            main_output_channels: main_out_channels,
            aux_input_count: aux_in_count,
            aux_output_count: aux_out_count,
        }
    }

    /// Validate that this configuration doesn't exceed system limits.
    ///
    /// Checks that:
    /// - Bus counts are within MAX_BUSES
    /// - Channel counts per bus are within MAX_CHANNELS
    ///
    /// Returns an error message if validation fails.
    pub fn validate(&self) -> Result<(), String> {
        // Check bus counts
        if self.input_bus_count > MAX_BUSES {
            return Err(format!(
                "Input bus count {} exceeds MAX_BUSES ({})",
                self.input_bus_count, MAX_BUSES
            ));
        }
        if self.output_bus_count > MAX_BUSES {
            return Err(format!(
                "Output bus count {} exceeds MAX_BUSES ({})",
                self.output_bus_count, MAX_BUSES
            ));
        }

        // Check channel counts
        for (i, bus) in self.input_buses.iter().enumerate() {
            if bus.channel_count > MAX_CHANNELS {
                return Err(format!(
                    "Input bus {} has {} channels, exceeds MAX_CHANNELS ({})",
                    i, bus.channel_count, MAX_CHANNELS
                ));
            }
        }
        for (i, bus) in self.output_buses.iter().enumerate() {
            if bus.channel_count > MAX_CHANNELS {
                return Err(format!(
                    "Output bus {} has {} channels, exceeds MAX_CHANNELS ({})",
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
            vec![BusInfo {
                channel_count: 2,
                bus_type: BusType::Main,
            }],
            vec![BusInfo {
                channel_count: 2,
                bus_type: BusType::Main,
            }],
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
    fn test_custom_config() {
        let config = CachedBusConfig::new(
            vec![
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Main,
                },
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Auxiliary,
                },
            ],
            vec![BusInfo {
                channel_count: 6,
                bus_type: BusType::Main,
            }],
        );

        assert_eq!(config.input_bus_count, 2);
        assert_eq!(config.output_bus_count, 1);
        assert_eq!(config.total_input_channels(), 4);
        assert_eq!(config.total_output_channels(), 6);
    }

    #[test]
    fn test_validation_success() {
        let config = CachedBusConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_to_bus_layout() {
        let config = CachedBusConfig::default();
        let layout = config.to_bus_layout();
        assert_eq!(layout.main_input_channels, 2);
        assert_eq!(layout.main_output_channels, 2);
        assert_eq!(layout.aux_input_count, 0);
        assert_eq!(layout.aux_output_count, 0);
    }

    #[test]
    fn test_bus_info_access() {
        let config = CachedBusConfig::default();
        assert!(config.input_bus_info(0).is_some());
        assert!(config.input_bus_info(1).is_none());
        assert_eq!(config.input_bus_info(0).unwrap().channel_count, 2);
    }
}
