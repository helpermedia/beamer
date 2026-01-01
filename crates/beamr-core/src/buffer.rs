//! Audio buffer abstractions.

/// Safe audio buffer abstraction for processing.
pub struct AudioBuffer<'a> {
    inputs: &'a [&'a [f32]],
    outputs: &'a mut [&'a mut [f32]],
    num_samples: usize,
}

impl<'a> AudioBuffer<'a> {
    /// Create a new audio buffer.
    ///
    /// # Safety
    /// The caller must ensure that input and output slices are valid for the
    /// specified number of samples.
    pub fn new(
        inputs: &'a [&'a [f32]],
        outputs: &'a mut [&'a mut [f32]],
        num_samples: usize,
    ) -> Self {
        Self {
            inputs,
            outputs,
            num_samples,
        }
    }

    /// Get the number of samples in this buffer.
    #[inline]
    pub fn num_samples(&self) -> usize {
        self.num_samples
    }

    /// Get the number of input channels.
    #[inline]
    pub fn num_input_channels(&self) -> usize {
        self.inputs.len()
    }

    /// Get the number of output channels.
    #[inline]
    pub fn num_output_channels(&self) -> usize {
        self.outputs.len()
    }

    /// Get an input channel's samples.
    ///
    /// Returns an empty slice if the channel doesn't exist.
    #[inline]
    pub fn input(&self, channel: usize) -> &[f32] {
        self.inputs
            .get(channel)
            .map(|c| &c[..self.num_samples])
            .unwrap_or(&[])
    }

    /// Get a mutable reference to an output channel's samples.
    ///
    /// # Panics
    /// Panics if the channel index is out of bounds.
    #[inline]
    pub fn output(&mut self, channel: usize) -> &mut [f32] {
        &mut self.outputs[channel][..self.num_samples]
    }

    /// Iterate over mutable output channel slices.
    #[inline]
    pub fn outputs_mut(&mut self) -> impl Iterator<Item = &mut [f32]> + use<'_, 'a> {
        let num_samples = self.num_samples;
        self.outputs.iter_mut().map(move |c| &mut c[..num_samples])
    }

    /// Copy input to output (passthrough).
    ///
    /// Only copies channels that exist in both input and output.
    pub fn copy_input_to_output(&mut self) {
        let num_channels = self.inputs.len().min(self.outputs.len());
        for ch in 0..num_channels {
            let input = &self.inputs[ch][..self.num_samples];
            let output = &mut self.outputs[ch][..self.num_samples];
            output.copy_from_slice(input);
        }
    }

    /// Clear all output channels to silence.
    pub fn clear_outputs(&mut self) {
        for output in self.outputs.iter_mut() {
            output[..self.num_samples].fill(0.0);
        }
    }
}
