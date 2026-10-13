//! A plugin in which two parameters have the same host id cannot be
//! created: the format wrappers refuse the instance.

#![cfg(feature = "derive")]

use beamer::core::parameter_types::check_unique_ids;
use beamer::core::preset::fnv1a_hash;
use beamer::prelude::*;
use beamer_vst3::factory::ComponentFactory;
use beamer_vst3::Vst3Processor;

static CONFIG: Config = Config::new("Clash", Category::Effect, "Bmer", "clsh");

#[derive(Parameters)]
struct InnerParameters {
    #[parameter(id = "ya", name = "Inner", default = 0.0, range = 0.0..=1.0)]
    value: FloatParameter,
}

/// Two parameters whose paths, `nbki` and `ot/ya`, have the same hash.
#[derive(Parameters)]
struct ClashParameters {
    #[parameter(id = "nbki", name = "Outer", default = 0.0, range = 0.0..=1.0)]
    value: FloatParameter,

    #[nested(group = "Inner")]
    ot: InnerParameters,
}

/// The same struct one level down, where the paths are `p/nbki` and
/// `p/ot/ya`. Their hashes differ.
#[derive(Parameters)]
struct NestedClashParameters {
    #[nested(group = "P")]
    p: ClashParameters,
}

struct Silence;

impl Processor for Silence {
    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {}
}

#[derive(Default, HasParameters)]
struct ClashDescriptor {
    #[parameters]
    parameters: Arc<ClashParameters>,
}

impl Descriptor for ClashDescriptor {
    type Setup = ();
    type Processor = Silence;

    fn prepare(&self, _: ()) -> Silence {
        Silence
    }
}

#[derive(Default, HasParameters)]
struct NestedClashDescriptor {
    #[parameters]
    parameters: Arc<NestedClashParameters>,
}

impl Descriptor for NestedClashDescriptor {
    type Setup = ();
    type Processor = Silence;

    fn prepare(&self, _: ()) -> Silence {
        Silence
    }
}

#[test]
fn test_the_two_paths_have_one_hash() {
    assert_eq!(fnv1a_hash("nbki"), fnv1a_hash("ot/ya"));
    assert_ne!(fnv1a_hash("p/nbki"), fnv1a_hash("p/ot/ya"));
}

#[test]
fn test_clash_is_reported_with_both_paths() {
    // Creating the parameters does not fail: only the root of a plugin
    // knows the ids that are in use.
    let parameters = ClashParameters::default();

    let error = check_unique_ids(&parameters).expect_err("the ids clash");
    assert!(error.contains("\"nbki\""), "{error}");
    assert!(error.contains("\"ot/ya\""), "{error}");
}

#[test]
fn test_no_clash_where_the_paths_differ() {
    let parameters = NestedClashParameters::default();
    assert!(check_unique_ids(&parameters).is_ok());
}

#[test]
fn test_vst3_refuses_an_instance_with_a_clash() {
    type Clash = Vst3Processor<ClashDescriptor, NoPresets<ClashParameters>>;
    type Fine = Vst3Processor<NestedClashDescriptor, NoPresets<NestedClashParameters>>;

    assert!(<Clash as ComponentFactory>::create(&CONFIG).is_none());
    assert!(<Fine as ComponentFactory>::create(&CONFIG).is_some());
}

#[cfg(target_os = "macos")]
#[test]
fn test_au_refuses_an_instance_with_a_clash() {
    use beamer_au::bridge::beamer_au_create_instance;
    use beamer_au::factory::register_factory;
    use beamer_au::{AuPluginInstance, AuProcessor};

    fn create() -> Box<dyn AuPluginInstance> {
        Box::new(AuProcessor::<ClashDescriptor, NoPresets<ClashParameters>>::new())
    }

    // One plugin per process: this test is the only one that registers.
    register_factory(create, &CONFIG);
    assert!(beamer_au_create_instance().is_null());
}
