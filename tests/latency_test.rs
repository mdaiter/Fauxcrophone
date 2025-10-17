use device_kit::latency::LatencyProbe;
use device_kit::ring::monotonic_timestamp_ns;
use device_kit::{AudioBuffer, Mixer};

const SAMPLE_RATE: u32 = 48_000;
const BLOCK_FRAMES: usize = 256;
const LATENCY_FRAMES: usize = 32;

#[test]
fn latency_compensation_inserts_expected_delay() {
    let mut mixer = Mixer::new(SAMPLE_RATE, BLOCK_FRAMES);
    let (handle, ring) = mixer.add_source(8_192);
    mixer.set_gain(handle, 1.0).unwrap();
    mixer.set_latency(handle, LATENCY_FRAMES as i32).unwrap();

    let probe = LatencyProbe::new(SAMPLE_RATE, 440.0, SAMPLE_RATE as usize / 20);
    let total_input_frames = SAMPLE_RATE as usize / 5; // 200 ms of audio.
    let mut input = vec![0.0f32; total_input_frames * 2];
    probe.emit_sine(440.0, &mut input);

    let mut recorded = Vec::new();
    let mut cursor = 0usize;

    while recorded.len() < (SAMPLE_RATE as usize / 10) * 2 {
        if cursor < input.len() {
            let end = (cursor + BLOCK_FRAMES * 2).min(input.len());
            let slice = &input[cursor..end];
            let mut remaining_frames = slice.len() / 2;
            let mut offset_samples = 0usize;
            while remaining_frames > 0 {
                let wrote = ring.push(&slice[offset_samples..], Some(monotonic_timestamp_ns()));
                if wrote == 0 {
                    recorded.extend(render_block(&mut mixer));
                    continue;
                }
                remaining_frames = remaining_frames.saturating_sub(wrote);
                offset_samples += wrote * 2;
            }
            cursor = end;
        }
        recorded.extend(render_block(&mut mixer));
        if cursor >= input.len() && ring.available_read() == 0 {
            break;
        }
    }

    // Ensure the initial latency window contains near-silence.
    let head_samples = LATENCY_FRAMES * 2;
    assert!(
        recorded.iter().take(head_samples).all(|&s| s.abs() < 1e-4),
        "expected initial {} frames to be silent, found {:?}",
        LATENCY_FRAMES,
        &recorded[..head_samples.min(16)]
    );

    let report = mixer.measure_latency(&recorded);
    assert!(
        (report.offset_frames as isize - LATENCY_FRAMES as isize).abs() <= 2,
        "measured latency {} differs from expected {}",
        report.offset_frames,
        LATENCY_FRAMES
    );
    assert!(
        report.correlation > 0.8,
        "correlation too low: {}",
        report.correlation
    );
}

fn render_block(mixer: &mut Mixer) -> Vec<f32> {
    let mut output = vec![0.0f32; BLOCK_FRAMES * 2];
    let mut buffer = AudioBuffer {
        data: output.as_mut_ptr(),
        frames: BLOCK_FRAMES as u32,
        channels: 2,
        timestamp_ns: 0,
    };
    mixer.process(&mut buffer).unwrap();
    output
}
