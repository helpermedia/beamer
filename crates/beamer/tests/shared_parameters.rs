//! A plugin has one set of parameters. The descriptor holds them for the
//! life of the instance and every processor it creates shares them.

#![cfg(feature = "derive")]

use beamer::prelude::*;

#[derive(Parameters)]
struct GainParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,
}

#[derive(Default, HasParameters)]
struct GainDescriptor {
    #[parameters]
    parameters: Arc<GainParameters>,
}

impl Descriptor for GainDescriptor {
    type Setup = SampleRate;
    type Processor = GainProcessor;

    fn prepare(&self, setup: SampleRate) -> GainProcessor {
        GainProcessor {
            parameters: Arc::clone(&self.parameters),
            sample_rate: setup.hz(),
        }
    }
}

struct GainProcessor {
    parameters: Arc<GainParameters>,
    sample_rate: f64,
}

impl Processor for GainProcessor {
    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {}
}

/// The parameter type the derive found for a descriptor.
fn parameters_of<D: HasParameters>(descriptor: &D) -> &Arc<D::Parameters> {
    descriptor.parameters()
}

#[test]
fn test_derive_finds_the_parameter_struct_inside_the_arc() {
    let descriptor = GainDescriptor::default();
    let parameters: &Arc<GainParameters> = parameters_of(&descriptor);

    assert!(Arc::ptr_eq(parameters, &descriptor.parameters));
}

#[test]
fn test_every_processor_shares_the_parameters_of_its_descriptor() {
    let descriptor = GainDescriptor::default();

    let first = descriptor.prepare(SampleRate(44100.0));
    descriptor.parameters.gain.set(0.25);
    assert_eq!(first.parameters.gain.get(), 0.25);

    // The host changes the sample rate: the first processor goes, the
    // descriptor and the parameters stay.
    drop(first);
    let second = descriptor.prepare(SampleRate(96000.0));
    assert_eq!(second.sample_rate, 96000.0);
    assert!(Arc::ptr_eq(&second.parameters, descriptor.parameters()));
    assert_eq!(second.parameters.gain.get(), 0.25);

    second.parameters.gain.set(0.75);
    assert_eq!(descriptor.parameters().gain.get(), 0.75);
}
