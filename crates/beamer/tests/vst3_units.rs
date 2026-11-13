//! The units a VST3 host sees for the parameter groups.
//!
//! Hosts such as Cubase show the units in the order of their ids, so the
//! wrapper numbers them by the position of their group in the plugin's
//! list, which follows the declaration.

#![cfg(feature = "derive")]

use beamer::prelude::*;
use beamer_vst3::vst3::Steinberg::kResultOk;
use beamer_vst3::vst3::Steinberg::Vst::{
    kNoParentUnitId, kRootUnitId, IEditControllerTrait, IUnitInfoTrait, ParameterInfo, UnitInfo,
};
use beamer_vst3::Vst3Processor;

static CONFIG: Config = Config::new("Units", Category::Effect, "Bmer", "unit");

#[beamer::group]
struct EnvelopeGroup {
    #[parameter(id = "attack", name = "Attack", default = 10.0, range = 0.1..=1000.0, kind = "ms")]
    attack: FloatParameter,
}

#[beamer::group]
struct SectionGroup {
    #[parameter(id = "level", name = "Level", default = 0.5, range = 0.0..=1.0)]
    level: FloatParameter,

    #[nested(group = "Envelope")]
    envelope: EnvelopeGroup,
}

/// Groups whose hashed ids are in another order than their declaration
/// ("drive" hashes lower than "Mixer"), with a flat group declared after a
/// nested one.
#[beamer::model]
struct UnitsModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    gain: FloatParameter,

    #[parameter(id = "mix", name = "Mix", default = 0.5, range = 0.0..=1.0, group = "Mixer")]
    mix: FloatParameter,

    #[nested(group = "Drive")]
    drive: SectionGroup,

    #[parameter(id = "out", name = "Out", default = 0.5, range = 0.0..=1.0, group = "Output")]
    out: FloatParameter,

    #[nested(group = "Tone")]
    tone: SectionGroup,
}

struct Silence;

impl Processor for Silence {
    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {}
}

impl Model for UnitsModel {
    type Setup = ();
    type Processor = Silence;

    fn prepare(self: Arc<Self>, _: ()) -> Silence {
        Silence
    }
}

fn text(chars: &[u16]) -> String {
    let units: Vec<u16> = chars.iter().take_while(|&&c| c != 0).copied().collect();
    String::from_utf16_lossy(&units)
}

#[test]
fn test_units_are_numbered_in_declaration_order() {
    assert!(group_id("drive") < group_id("Mixer"));
    let processor = Vst3Processor::<UnitsModel>::new(&CONFIG);

    // SAFETY: the processor lives for the whole block; `info` is a
    // writable local the call fills in.
    let units: Vec<(i32, i32, String)> = unsafe {
        (0..processor.getUnitCount())
            .map(|index| {
                let mut info: UnitInfo = std::mem::zeroed();
                assert_eq!(processor.getUnitInfo(index, &mut info), kResultOk);
                (info.id, info.parentUnitId, text(&info.name))
            })
            .collect()
    };
    let expected = [
        (kRootUnitId, kNoParentUnitId, ""),
        (1, kRootUnitId, "Mixer"),
        (2, kRootUnitId, "Drive"),
        (3, 2, "Envelope"),
        (4, kRootUnitId, "Output"),
        (5, kRootUnitId, "Tone"),
        (6, 5, "Envelope"),
    ];
    let expected: Vec<(i32, i32, String)> =
        expected.iter().map(|&(id, parent, name)| (id, parent, name.to_string())).collect();
    assert_eq!(units, expected);
}

#[test]
fn test_parameters_are_in_the_units_of_their_groups() {
    let processor = Vst3Processor::<UnitsModel>::new(&CONFIG);

    // SAFETY: the processor lives for the whole block; `info` is a
    // writable local the call fills in.
    let parameters: Vec<(String, i32)> = unsafe {
        (0..7)
            .map(|index| {
                let mut info: ParameterInfo = std::mem::zeroed();
                assert_eq!(processor.getParameterInfo(index, &mut info), kResultOk);
                (text(&info.title), info.unitId)
            })
            .collect()
    };
    // The parameters come in the order of the fields, as the units do.
    let expected = [
        ("Gain", 0),
        ("Mix", 1),
        ("Level", 2),
        ("Attack", 3),
        ("Out", 4),
        ("Level", 5),
        ("Attack", 6),
    ];
    let expected: Vec<(String, i32)> = expected.iter().map(|&(name, unit)| (name.to_string(), unit)).collect();
    assert_eq!(parameters, expected);
}
