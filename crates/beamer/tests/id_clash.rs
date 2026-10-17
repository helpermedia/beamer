//! A plugin in which two parameters have the same host id cannot be
//! created: the format wrappers refuse the instance.

#![cfg(feature = "derive")]

use beamer::core::parameter_types::check_unique_ids;
use beamer::core::preset::fnv1a_hash;
use beamer::prelude::*;
use beamer_vst3::factory::ComponentFactory;
use beamer_vst3::Vst3Processor;

static CONFIG: Config = Config::new("Clash", Category::Effect, "Bmer", "clsh");

#[beamer::group]
struct InnerGroup {
    #[parameter(id = "ya", name = "Inner", default = 0.0, range = 0.0..=1.0)]
    value: FloatParameter,
}

/// Two parameters whose paths, `nbki` and `ot/ya`, have the same hash.
#[beamer::model]
struct ClashModel {
    #[parameter(id = "nbki", name = "Outer", default = 0.0, range = 0.0..=1.0)]
    value: FloatParameter,

    #[nested(group = "Inner")]
    ot: InnerGroup,
}

/// The same struct one level down, where the paths are `p/nbki` and
/// `p/ot/ya`. Their hashes differ. A model generates the same code as a
/// group, so it can sit in another model as a nested group.
#[beamer::model]
struct NestedClashModel {
    #[nested(group = "P")]
    p: ClashModel,
}

struct Silence;

impl Processor for Silence {
    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {}
}

impl Model for ClashModel {
    type Setup = ();
    type Processor = Silence;

    fn prepare(self: Arc<Self>, _: ()) -> Silence {
        Silence
    }
}

impl Model for NestedClashModel {
    type Setup = ();
    type Processor = Silence;

    fn prepare(self: Arc<Self>, _: ()) -> Silence {
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
    // Creating the model does not fail: only the root of a plugin knows the
    // ids that are in use.
    let model = ClashModel::default();

    let error = check_unique_ids(&model).expect_err("the ids clash");
    assert!(error.contains("\"nbki\""), "{error}");
    assert!(error.contains("\"ot/ya\""), "{error}");
}

#[test]
fn test_no_clash_where_the_paths_differ() {
    let model = NestedClashModel::default();
    assert!(check_unique_ids(&model).is_ok());
}

#[test]
fn test_vst3_refuses_an_instance_with_a_clash() {
    type Clash = Vst3Processor<ClashModel, NoPresets<ClashModel>>;
    type Fine = Vst3Processor<NestedClashModel, NoPresets<NestedClashModel>>;

    assert!(<Clash as ComponentFactory>::create(&CONFIG).is_none());
    assert!(<Fine as ComponentFactory>::create(&CONFIG).is_some());
}

/// A parameter whose id, the hash of `hvcbaeyv`, is the one the VST3
/// wrapper uses for the parameter of MIDI CC 7.
#[beamer::model]
struct MidiCcIdModel {
    #[parameter(id = "hvcbaeyv", name = "Volume", default = 0.0, range = 0.0..=1.0)]
    value: FloatParameter,
}

/// A parameter whose id, the hash of `ukaqaxdz`, is the one the VST3
/// wrapper uses for its program change parameter.
#[beamer::model]
struct ProgramIdModel {
    #[parameter(id = "ukaqaxdz", name = "Program", default = 0.0, range = 0.0..=1.0)]
    value: FloatParameter,
}

impl Model for MidiCcIdModel {
    type Setup = ();
    type Processor = Silence;

    fn prepare(self: Arc<Self>, _: ()) -> Silence {
        Silence
    }
}

impl Model for ProgramIdModel {
    type Setup = ();
    type Processor = Silence;

    fn prepare(self: Arc<Self>, _: ()) -> Silence {
        Silence
    }
}

#[test]
fn test_vst3_refuses_an_id_it_uses_itself() {
    type MidiCcId = Vst3Processor<MidiCcIdModel, NoPresets<MidiCcIdModel>>;
    type ProgramId = Vst3Processor<ProgramIdModel, NoPresets<ProgramIdModel>>;

    assert_eq!(fnv1a_hash("hvcbaeyv"), 0x1000_0007);
    assert_eq!(fnv1a_hash("ukaqaxdz"), 0x2000_0000);

    // The ids are unique within each plugin.
    assert!(check_unique_ids(&MidiCcIdModel::default()).is_ok());
    assert!(check_unique_ids(&ProgramIdModel::default()).is_ok());

    assert!(<MidiCcId as ComponentFactory>::create(&CONFIG).is_none());
    assert!(<ProgramId as ComponentFactory>::create(&CONFIG).is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn test_au_refuses_an_instance_with_a_clash() {
    use beamer_au::bridge::beamer_au_create_instance;
    use beamer_au::factory::register_factory;
    use beamer_au::{AuPluginInstance, AuProcessor};

    fn create() -> Box<dyn AuPluginInstance> {
        Box::new(AuProcessor::<ClashModel, NoPresets<ClashModel>>::new())
    }

    // One plugin per process: this test is the only one that registers.
    register_factory(create, &CONFIG);
    assert!(beamer_au_create_instance().is_null());
}
