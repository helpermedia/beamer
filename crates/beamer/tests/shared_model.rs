//! A plugin has one model. The framework holds it in an `Arc` for the life
//! of the instance and every processor it creates shares it.

#![cfg(feature = "derive")]

use beamer::prelude::*;

#[beamer::model]
struct GainModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,
}

impl Model for GainModel {
    type Setup = SampleRate;
    type Processor = GainProcessor;

    fn prepare(self: Arc<Self>, setup: SampleRate) -> GainProcessor {
        GainProcessor {
            sample_rate: setup.hz(),
            model: self,
        }
    }
}

struct GainProcessor {
    model: Arc<GainModel>,
    sample_rate: f64,
}

impl Processor for GainProcessor {
    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {}
}

#[test]
fn test_every_processor_shares_the_model() {
    // What a format wrapper does: one model in an `Arc`, a clone of it for
    // each processor.
    let model = Arc::new(GainModel::default());

    let first = Arc::clone(&model).prepare(SampleRate(44100.0));
    model.gain.set(0.25);
    assert_eq!(first.model.gain.get(), 0.25);

    // The host changes the sample rate: the first processor goes, the model
    // stays.
    drop(first);
    let second = Arc::clone(&model).prepare(SampleRate(96000.0));
    assert_eq!(second.sample_rate, 96000.0);
    assert!(Arc::ptr_eq(&second.model, &model));
    assert_eq!(second.model.gain.get(), 0.25);

    second.model.gain.set(0.75);
    assert_eq!(model.gain.get(), 0.75);
}
