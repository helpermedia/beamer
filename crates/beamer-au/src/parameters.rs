//! Parameter tree construction and synchronization for Audio Unit.
//!
//! This module provides functions for building and managing `AUParameterTree`
//! from Beamer's `ParameterStore`, enabling AU hosts to display and automate
//! plugin parameters.
//!
//! # Parameter Synchronization
//!
//! Parameters are synchronized bidirectionally between the plugin and AU:
//! - **Provider callbacks** (AU → Plugin): When the AU reads parameter values
//! - **Observer callbacks** (Plugin → AU): When the AU changes parameter values
//!
//! This ensures the parameter tree stays in sync with the plugin's internal state
//! and responds to host automation.
//!
//! # Unit Mapping
//!
//! Beamer parameter units (e.g., "dB", "Hz", "ms") are mapped to AU parameter
//! units (e.g., Decibels, Hertz, Milliseconds) for proper display and scaling
//! by the AU host.

use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{class, msg_send};
use objc2_foundation::{NSArray, NSNumber, NSString};

use crate::instance::AuPluginInstance;

/// AU parameter unit types.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AUParameterUnit {
    Generic = 0,
    Indexed = 1,
    Boolean = 2,
    Percent = 3,
    Seconds = 4,
    SampleFrames = 5,
    Phase = 6,
    Rate = 7,
    Hertz = 8,
    Cents = 9,
    RelativeSemiTones = 10,
    MIDINoteNumber = 11,
    MIDIController = 12,
    Decibels = 13,
    LinearGain = 14,
    Degrees = 15,
    EqualPowerCrossfade = 16,
    MixerFaderCurve1 = 17,
    Pan = 18,
    Meters = 19,
    AbsoluteCents = 20,
    Octaves = 21,
    BPM = 22,
    Beats = 23,
    Milliseconds = 24,
    Ratio = 25,
    CustomUnit = 26,
}

/// Map beamer parameter units to AU units.
pub fn map_parameter_unit(units: &str) -> AUParameterUnit {
    match units.to_lowercase().as_str() {
        "db" | "decibels" => AUParameterUnit::Decibels,
        "hz" | "hertz" => AUParameterUnit::Hertz,
        "ms" | "milliseconds" => AUParameterUnit::Milliseconds,
        "s" | "seconds" => AUParameterUnit::Seconds,
        "%" | "percent" => AUParameterUnit::Percent,
        "pan" => AUParameterUnit::Pan,
        "ratio" => AUParameterUnit::Ratio,
        "bpm" => AUParameterUnit::BPM,
        "semitones" => AUParameterUnit::RelativeSemiTones,
        "cents" => AUParameterUnit::Cents,
        "octaves" => AUParameterUnit::Octaves,
        "degrees" => AUParameterUnit::Degrees,
        _ => AUParameterUnit::Generic,
    }
}

/// AU parameter flags.
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum AUParameterFlags {
    /// Parameter value can be changed at any time
    Readable = 1 << 0,
    /// Parameter value can be read at any time
    Writable = 1 << 1,
    /// Parameter can be automated
    MeterReadOnly = 1 << 2,
    /// Default flags for most parameters
    Default = (1 << 0) | (1 << 1), // Readable | Writable
}

/// Build an AUParameterTree from a ParameterStore.
///
/// This creates the parameter tree structure that AU hosts use to
/// display and automate plugin parameters.
///
/// # Arguments
///
/// * `plugin` - Thread-safe reference to the plugin instance
///
/// # Returns
///
/// An Objective-C AUParameterTree instance, or None if creation fails.
pub fn build_parameter_tree(
    plugin: Arc<Mutex<Box<dyn AuPluginInstance>>>,
) -> Option<Retained<AnyObject>> {
    // Get parameter info from the plugin
    let plugin_guard = plugin.lock().ok()?;
    let store = plugin_guard.parameter_store().ok()?;
    let param_count = store.count();

    if param_count == 0 {
        // Return empty parameter tree
        return create_empty_parameter_tree();
    }

    // Create AUParameter objects for each parameter
    let mut parameters: Vec<Retained<AnyObject>> = Vec::with_capacity(param_count);

    for i in 0..param_count {
        if let Some(info) = store.info(i) {
            if let Some(param) = create_au_parameter(
                info.id,
                info.name,
                info.units,
                0.0, // min (normalized)
                1.0, // max (normalized)
                info.default_normalized,
                info.step_count,
                Arc::clone(&plugin),
            ) {
                parameters.push(param);
            }
        }
    }

    // Create the parameter tree from the parameters array
    create_parameter_tree_from_params(&parameters)
}

/// Create a single AUParameter.
#[allow(clippy::too_many_arguments)]
fn create_au_parameter(
    id: u32,
    name: &str,
    units: &str,
    min_value: f64,
    max_value: f64,
    default_value: f64,
    step_count: i32,
    plugin: Arc<Mutex<Box<dyn AuPluginInstance>>>,
) -> Option<Retained<AnyObject>> {
    // SAFETY: All ObjC messaging uses valid class references and properly retained
    // objects. NSString instances are kept alive by Retained<>. The null pointers
    // for unitName, valueStrings, and dependentParameters are valid per AU API.
    unsafe {
        // Get AUParameterTree class
        let tree_class: &AnyClass = class!(AUParameterTree);

        // Create identifier string from ID
        let identifier = NSString::from_str(&format!("param_{}", id));
        let display_name = NSString::from_str(name);

        // Determine AU unit
        let au_unit = map_parameter_unit(units);

        // Determine flags
        let flags: u32 = if step_count == 1 {
            // Boolean parameter
            AUParameterFlags::Default as u32
        } else {
            AUParameterFlags::Default as u32
        };

        // Create min/max/default NSNumbers
        let _min_num: Retained<NSNumber> = NSNumber::new_f32(min_value as f32);
        let _max_num: Retained<NSNumber> = NSNumber::new_f32(max_value as f32);

        // Call AUParameterTree.createParameterWithIdentifier:name:address:min:max:unit:unitName:flags:valueStrings:dependentParameters:
        // This is a complex class method, so we'll use a simpler approach

        // Try the simpler createParameter method
        let param: Option<Retained<AnyObject>> = msg_send![
            tree_class,
            createParameterWithIdentifier: &*identifier,
            name: &*display_name,
            address: id as u64,
            min: min_value as f32,
            max: max_value as f32,
            unit: au_unit as i32,
            unitName: std::ptr::null::<NSString>(),
            flags: flags,
            valueStrings: std::ptr::null::<NSArray<NSString>>(),
            dependentParameters: std::ptr::null::<NSArray<NSNumber>>()
        ];

        if let Some(ref param) = param {
            // Set the default value
            let _: () = msg_send![param, setValue: default_value as f32];

            // Set up value observer (getter/setter callbacks)
            setup_parameter_callbacks(param, id, Arc::clone(&plugin));
        }

        param
    }
}

/// Set up parameter value callbacks for bidirectional sync.
fn setup_parameter_callbacks(
    param: &AnyObject,
    param_id: u32,
    plugin: Arc<Mutex<Box<dyn AuPluginInstance>>>,
) {
    // SAFETY: ObjC messaging to set callback blocks on AUParameter. The RcBlock
    // captures the plugin Arc by value, ensuring the plugin lives as long as the
    // callback. The blocks are reference counted and released when the AU is deallocated.
    unsafe {
        // Create value provider (AU -> Plugin read)
        // This is called when the AU needs to read the current parameter value from the plugin
        let plugin_for_provider = Arc::clone(&plugin);
        let provider = RcBlock::new(move |_: *const AnyObject| -> f32 {
            if let Ok(guard) = plugin_for_provider.lock() {
                if let Ok(store) = guard.parameter_store() {
                    store.get_normalized(param_id) as f32
                } else {
                    0.0
                }
            } else {
                0.0
            }
        });

        // Create value observer (Plugin -> AU write)
        // This is called when the AU changes the parameter value and needs to update the plugin
        let plugin_for_observer = Arc::clone(&plugin);
        let observer = RcBlock::new(move |_: *const AnyObject, value: f32| {
            if let Ok(mut guard) = plugin_for_observer.lock() {
                if let Ok(store) = guard.parameter_store_mut() {
                    store.set_normalized(param_id, value as f64);
                }
            }
        });

        // Set the callbacks on the AUParameter
        let _: () = msg_send![param, setImplementorValueProvider: &*provider];
        let _: () = msg_send![param, setImplementorValueObserver: &*observer];
    }
}

/// Create an empty parameter tree.
fn create_empty_parameter_tree() -> Option<Retained<AnyObject>> {
    // SAFETY: Standard ObjC messaging to create an AUParameterTree with empty children.
    unsafe {
        let tree_class: &AnyClass = class!(AUParameterTree);
        let empty_array: Retained<NSArray<AnyObject>> = NSArray::new();
        msg_send![tree_class, createTreeWithChildren: &*empty_array]
    }
}

/// Create a parameter tree from an array of parameters.
fn create_parameter_tree_from_params(
    parameters: &[Retained<AnyObject>],
) -> Option<Retained<AnyObject>> {
    // SAFETY: ObjC messaging to create an AUParameterTree. The parameters slice
    // contains properly retained AUParameter objects that remain valid during
    // NSArray construction and tree creation.
    unsafe {
        let tree_class: &AnyClass = class!(AUParameterTree);

        // Convert to NSArray
        let ns_array = NSArray::from_retained_slice(parameters);

        // Create tree with children
        msg_send![tree_class, createTreeWithChildren: &*ns_array]
    }
}

/// Sync parameter values from AU to plugin.
///
/// Called by the AU when a parameter value changes (automation, UI, etc.).
pub fn sync_parameter_to_plugin(plugin: &mut Box<dyn AuPluginInstance>, param_id: u32, value: f32) {
    if let Ok(store) = plugin.parameter_store_mut() {
        store.set_normalized(param_id, value as f64);
    }
}

/// Get parameter value from plugin for AU.
///
/// Called by the AU when it needs the current parameter value.
pub fn get_parameter_from_plugin(plugin: &dyn AuPluginInstance, param_id: u32) -> f32 {
    if let Ok(store) = plugin.parameter_store() {
        store.get_normalized(param_id) as f32
    } else {
        0.0
    }
}
