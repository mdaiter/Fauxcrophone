//! Latency probe utilities and sine-wave generators for self-test loops.

use crate::ring::monotonic_timestamp_ns;

/// Latency measurement report capturing the best-fit offset.
#[derive(Debug, Clone, Copy)]
pub struct LatencyReport {
    /// Estimated offset in frames between reference and recorded data.
    pub offset_frames: usize,
    /// Offset converted to seconds.
    pub offset_seconds: f32,
    /// Normalized cross-correlation score (0.0 - 1.0).
    pub correlation: f32,
    /// Timestamp captured when the report was generated.
    pub measured_at_ns: u64,
}

/// Deterministic sine generator and latency estimator.
pub struct LatencyProbe {
    sample_rate: u32,
    default_frequency: f32,
    reference: Vec<f32>,
}

impl LatencyProbe {
    /// Create a new probe with a default sine reference.
    pub fn new(sample_rate: u32, default_frequency: f32, window_frames: usize) -> Self {
        let reference = build_reference_sine(sample_rate, default_frequency, window_frames);
        Self {
            sample_rate,
            default_frequency,
            reference,
        }
    }

    /// Render a sine wave into the provided buffer. Returns frames written.
    pub fn emit_sine(&self, frequency_hz: f32, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;
        if frames == 0 {
            return 0;
        }
        let freq = if frequency_hz > 0.0 {
            frequency_hz
        } else {
            self.default_frequency
        };
        write_sine(freq, self.sample_rate, out);
        frames
    }

    /// Compute latency by correlating the recorded stereo buffer with the stored reference.
    pub fn measure(&self, recorded: &[f32]) -> LatencyReport {
        let recorded_frames = recorded.len() / 2;
        let reference_frames = self.reference.len() / 2;
        if recorded_frames == 0 || reference_frames == 0 {
            return LatencyReport {
                offset_frames: 0,
                offset_seconds: 0.0,
                correlation: 0.0,
                measured_at_ns: monotonic_timestamp_ns(),
            };
        }

        let max_offset = recorded_frames.saturating_sub(reference_frames);
        if max_offset == 0 {
            return self.single_slice_report(recorded);
        }

        let reference_norm = energy(&self.reference);
        let mut best = (0usize, 0.0f32);

        for offset in 0..=max_offset {
            let slice_start = offset * 2;
            let slice_end = slice_start + self.reference.len();
            let recorded_slice = &recorded[slice_start..slice_end];
            let corr = correlation(&self.reference, recorded_slice, reference_norm);
            if corr > best.1 {
                best = (offset, corr);
            }
        }

        LatencyReport {
            offset_frames: best.0,
            offset_seconds: best.0 as f32 / self.sample_rate as f32,
            correlation: best.1,
            measured_at_ns: monotonic_timestamp_ns(),
        }
    }

    fn single_slice_report(&self, recorded: &[f32]) -> LatencyReport {
        let reference_norm = energy(&self.reference);
        let recorded_norm = energy(recorded);
        let corr = if reference_norm > 0.0 && recorded_norm > 0.0 {
            dot(&self.reference, recorded) / (reference_norm * recorded_norm)
        } else {
            0.0
        };
        LatencyReport {
            offset_frames: 0,
            offset_seconds: 0.0,
            correlation: corr,
            measured_at_ns: monotonic_timestamp_ns(),
        }
    }
}

fn build_reference_sine(sample_rate: u32, frequency: f32, frames: usize) -> Vec<f32> {
    let mut buffer = vec![0.0f32; frames * 2];
    write_sine(frequency, sample_rate, &mut buffer);
    buffer
}

fn write_sine(frequency: f32, sample_rate: u32, out: &mut [f32]) {
    let step = frequency / sample_rate as f32;
    let mut phase = 0.0f32;
    for frame in out.chunks_exact_mut(2) {
        let value = (phase * std::f32::consts::TAU).sin() * 0.5;
        frame[0] = value;
        frame[1] = value;
        phase = (phase + step).fract();
    }
}

fn correlation(reference: &[f32], recorded: &[f32], reference_norm: f32) -> f32 {
    let recorded_norm = energy(recorded);
    if reference_norm == 0.0 || recorded_norm == 0.0 {
        return 0.0;
    }
    dot(reference, recorded) / (reference_norm * recorded_norm)
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

fn energy(buf: &[f32]) -> f32 {
    buf.iter().map(|x| x * x).sum::<f32>().sqrt()
}
