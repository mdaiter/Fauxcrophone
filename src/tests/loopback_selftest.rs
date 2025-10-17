use std::f32::consts::TAU;

use crate::{AudioBuffer, Mixer, monotonic_timestamp_ns};

#[test]
fn loopback_selftest_sine_through_mixer() {
    let sample_rate = 48_000u32;
    let block_frames = 256usize;
    let mut mixer = Mixer::new(sample_rate, block_frames);
    let (_handle, ring) = mixer.add_source(block_frames * 8);

    let frequency_hz = 1_000.0f32;
    let total_frames = (sample_rate / 10) as usize; // 100ms
    let mut input: Vec<f32> = Vec::with_capacity(total_frames * 2);

    for n in 0..total_frames {
        let phase = frequency_hz * n as f32 / sample_rate as f32;
        let sample = (phase * TAU).sin() * 0.5;
        input.push(sample);
        input.push(sample);
    }

    let mut recorded = Vec::with_capacity(input.len());
    for chunk in input.chunks(block_frames * 2) {
        let timestamp = monotonic_timestamp_ns();
        let frames_in_chunk = chunk.len() / 2;
        ring.push(chunk, Some(timestamp));

        let mut buffer = vec![0.0f32; block_frames * 2];
        let mut audio = AudioBuffer {
            data: buffer.as_mut_ptr(),
            frames: block_frames as u32,
            channels: 2,
            timestamp_ns: timestamp,
        };

        mixer.process(&mut audio).expect("process block");
        recorded.extend_from_slice(&buffer[..frames_in_chunk * 2]);
    }

    assert_eq!(recorded.len(), input.len());

    let expected_rms = rms(&input);
    let actual_rms = rms(&recorded);
    let amplitude_error = (expected_rms - actual_rms).abs();
    assert!(
        amplitude_error < 0.05,
        "RMS mismatch: expected {expected_rms}, got {actual_rms}"
    );

    let corr = correlation(&input, &recorded);
    assert!(corr > 0.95, "phase correlation too low: {corr}");
}

fn rms(signal: &[f32]) -> f32 {
    let energy: f32 = signal.iter().map(|s| s * s).sum();
    (energy / signal.len() as f32).sqrt()
}

fn correlation(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f64;
    let mut energy_a = 0.0f64;
    let mut energy_b = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += *x as f64 * *y as f64;
        energy_a += (*x as f64).powi(2);
        energy_b += (*y as f64).powi(2);
    }
    if energy_a == 0.0 || energy_b == 0.0 {
        0.0
    } else {
        (dot / (energy_a.sqrt() * energy_b.sqrt())) as f32
    }
}
