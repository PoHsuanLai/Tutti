//! Delay buffer for latency compensation.

use crate::compat::Vec;

pub struct DelayBuffer {
    left_buffer: Vec<f32>,
    right_buffer: Vec<f32>,
    write_pos: usize,
    delay_samples: usize,
}

impl DelayBuffer {
    pub fn new(delay_samples: usize) -> Self {
        Self {
            left_buffer: vec![0.0; delay_samples.max(1)],
            right_buffer: vec![0.0; delay_samples.max(1)],
            write_pos: 0,
            delay_samples,
        }
    }

    #[inline]
    pub fn process_batch(
        &mut self,
        left_in: &[f32],
        right_in: &[f32],
        left_out: &mut [f32],
        right_out: &mut [f32],
    ) -> usize {
        let nframes = left_in
            .len()
            .min(right_in.len())
            .min(left_out.len())
            .min(right_out.len());

        if self.delay_samples == 0 {
            left_out[..nframes].copy_from_slice(&left_in[..nframes]);
            right_out[..nframes].copy_from_slice(&right_in[..nframes]);
            return nframes;
        }

        let buffer_len = self.left_buffer.len();

        left_out[..nframes]
            .iter_mut()
            .zip(right_out[..nframes].iter_mut())
            .zip(left_in[..nframes].iter())
            .zip(right_in[..nframes].iter())
            .for_each(|(((l_out, r_out), &l_in), &r_in)| {
                let read_pos = if self.write_pos >= self.delay_samples {
                    self.write_pos - self.delay_samples
                } else {
                    buffer_len + self.write_pos - self.delay_samples
                };

                *l_out = self.left_buffer[read_pos];
                *r_out = self.right_buffer[read_pos];

                self.left_buffer[self.write_pos] = l_in;
                self.right_buffer[self.write_pos] = r_in;

                self.write_pos = (self.write_pos + 1) % buffer_len;
            });

        nframes
    }

    #[inline]
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        if self.delay_samples == 0 {
            return (left, right);
        }

        // Calculate read position (circular buffer)
        let read_pos = if self.write_pos >= self.delay_samples {
            self.write_pos - self.delay_samples
        } else {
            self.left_buffer.len() + self.write_pos - self.delay_samples
        };

        // Read delayed samples
        let delayed_left = self.left_buffer[read_pos];
        let delayed_right = self.right_buffer[read_pos];

        // Write new samples
        self.left_buffer[self.write_pos] = left;
        self.right_buffer[self.write_pos] = right;

        // Advance write position
        self.write_pos = (self.write_pos + 1) % self.left_buffer.len();

        (delayed_left, delayed_right)
    }

    pub fn delay_samples(&self) -> usize {
        self.delay_samples
    }

    pub fn set_delay(&mut self, new_delay_samples: usize) {
        if new_delay_samples == self.delay_samples {
            return;
        }

        self.delay_samples = new_delay_samples;
        let buffer_size = new_delay_samples.max(1);

        self.left_buffer.resize(buffer_size, 0.0);
        self.right_buffer.resize(buffer_size, 0.0);
        self.write_pos = 0;

        self.clear();
    }

    pub fn clear(&mut self) {
        self.left_buffer.fill(0.0);
        self.right_buffer.fill(0.0);
        self.write_pos = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delay_buffer_creation() {
        let buffer = DelayBuffer::new(100);
        assert_eq!(buffer.delay_samples(), 100);
    }

    #[test]
    fn test_zero_delay() {
        let mut buffer = DelayBuffer::new(0);
        let (left, right) = buffer.process(1.0, 0.5);
        assert_eq!(left, 1.0);
        assert_eq!(right, 0.5);
    }

    #[test]
    fn test_delay_processing() {
        let mut buffer = DelayBuffer::new(3);

        // First 3 samples should be silent (buffer is empty)
        assert_eq!(buffer.process(1.0, 1.0), (0.0, 0.0));
        assert_eq!(buffer.process(2.0, 2.0), (0.0, 0.0));
        assert_eq!(buffer.process(3.0, 3.0), (0.0, 0.0));

        // Now we should get the delayed samples
        assert_eq!(buffer.process(4.0, 4.0), (1.0, 1.0));
        assert_eq!(buffer.process(5.0, 5.0), (2.0, 2.0));
        assert_eq!(buffer.process(6.0, 6.0), (3.0, 3.0));
    }

    #[test]
    fn test_delay_resize() {
        let mut buffer = DelayBuffer::new(2);

        buffer.process(1.0, 1.0);
        buffer.process(2.0, 2.0);

        // Resize to larger delay
        buffer.set_delay(5);
        assert_eq!(buffer.delay_samples(), 5);

        // Buffer should be cleared
        assert_eq!(buffer.process(3.0, 3.0), (0.0, 0.0));
    }

    #[test]
    fn test_clear() {
        let mut buffer = DelayBuffer::new(2);

        buffer.process(1.0, 1.0);
        buffer.process(2.0, 2.0);

        buffer.clear();

        // Should output silence after clear
        assert_eq!(buffer.process(3.0, 3.0), (0.0, 0.0));
    }

    // ===== Batch Processing Tests =====

    #[test]
    fn test_batch_zero_delay() {
        let mut buffer = DelayBuffer::new(0);

        let left_in = vec![1.0, 2.0, 3.0, 4.0];
        let right_in = vec![0.5, 1.0, 1.5, 2.0];
        let mut left_out = vec![0.0; 4];
        let mut right_out = vec![0.0; 4];

        let nframes = buffer.process_batch(&left_in, &right_in, &mut left_out, &mut right_out);

        assert_eq!(nframes, 4);
        assert_eq!(left_out, left_in, "Zero delay should passthrough left");
        assert_eq!(right_out, right_in, "Zero delay should passthrough right");
    }

    #[test]
    fn test_batch_delay_processing() {
        let mut buffer = DelayBuffer::new(3);

        // First batch: should output silence (buffer is empty)
        let left_in1 = vec![1.0, 2.0, 3.0];
        let right_in1 = vec![0.1, 0.2, 0.3];
        let mut left_out1 = vec![999.0; 3];
        let mut right_out1 = vec![999.0; 3];

        buffer.process_batch(&left_in1, &right_in1, &mut left_out1, &mut right_out1);

        assert_eq!(
            left_out1,
            vec![0.0, 0.0, 0.0],
            "First 3 samples should be silent"
        );
        assert_eq!(right_out1, vec![0.0, 0.0, 0.0]);

        // Second batch: should output the delayed samples from first batch
        let left_in2 = vec![4.0, 5.0, 6.0];
        let right_in2 = vec![0.4, 0.5, 0.6];
        let mut left_out2 = vec![0.0; 3];
        let mut right_out2 = vec![0.0; 3];

        buffer.process_batch(&left_in2, &right_in2, &mut left_out2, &mut right_out2);

        assert_eq!(left_out2, vec![1.0, 2.0, 3.0], "Should get delayed samples");
        assert_eq!(right_out2, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn test_batch_vs_scalar_equivalence() {
        // Verify that batch and scalar methods produce identical results

        let mut buffer_batch = DelayBuffer::new(5);
        let mut buffer_scalar = DelayBuffer::new(5);

        // Test data
        let left_in = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let right_in = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];

        // Process with batch method
        let mut left_batch = vec![0.0; 8];
        let mut right_batch = vec![0.0; 8];
        buffer_batch.process_batch(&left_in, &right_in, &mut left_batch, &mut right_batch);

        // Process with scalar method
        let mut left_scalar = [0.0; 8];
        let mut right_scalar = [0.0; 8];
        for i in 0..8 {
            let (l, r) = buffer_scalar.process(left_in[i], right_in[i]);
            left_scalar[i] = l;
            right_scalar[i] = r;
        }

        // Compare results
        for i in 0..8 {
            assert_eq!(
                left_batch[i], left_scalar[i],
                "Sample {} left mismatch: batch={} scalar={}",
                i, left_batch[i], left_scalar[i]
            );
            assert_eq!(
                right_batch[i], right_scalar[i],
                "Sample {} right mismatch: batch={} scalar={}",
                i, right_batch[i], right_scalar[i]
            );
        }
    }

    #[test]
    fn test_batch_partial_buffers() {
        let mut buffer = DelayBuffer::new(2);

        // Test with different sized buffers - should use minimum size
        let left_in = vec![1.0, 2.0, 3.0, 4.0, 5.0]; // 5 samples
        let right_in = vec![0.1, 0.2]; // 2 samples
        let mut left_out = vec![0.0; 10];
        let mut right_out = vec![0.0; 3];

        let nframes = buffer.process_batch(&left_in, &right_in, &mut left_out, &mut right_out);

        // Should process minimum size (2 samples)
        assert_eq!(nframes, 2);
    }

    #[test]
    fn test_batch_circular_wraparound() {
        // Test that circular buffer wraps correctly with batch processing
        let mut buffer = DelayBuffer::new(3);

        // Fill buffer
        let data1 = vec![1.0, 2.0, 3.0];
        let mut left_out_dummy = vec![0.0; 3];
        let mut right_out_dummy = vec![0.0; 3];
        buffer.process_batch(&data1, &data1, &mut left_out_dummy, &mut right_out_dummy);

        // Process enough to wrap around multiple times
        let data2 = vec![4.0, 5.0, 6.0, 7.0, 8.0];
        let mut left_out = vec![0.0; 5];
        let mut right_out = vec![0.0; 5];

        buffer.process_batch(&data2, &data2, &mut left_out, &mut right_out);

        // Verify delayed output (should get samples from data1)
        assert_eq!(left_out[0], 1.0);
        assert_eq!(left_out[1], 2.0);
        assert_eq!(left_out[2], 3.0);
        assert_eq!(left_out[3], 4.0);
        assert_eq!(left_out[4], 5.0);
    }
}
