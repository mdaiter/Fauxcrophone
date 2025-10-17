use std::f32::consts::TAU;

use coreaudio_sys::{AudioBuffer, AudioBufferList, AudioTimeStamp, kAudioTimeStampHostTimeValid};

use device_kit::{
    LoopbackRenderArgs, device_kit_monotonic_time_ns, loopback_mixer_create,
    loopback_mixer_destroy, loopback_mixer_process, loopback_mixer_push_node_frames,
    loopback_mixer_register_node_source, loopback_mixer_set_node_gain, loopback_mixer_submit_input,
};

const SAMPLE_RATE: f64 = 48_000.0;
const BLOCK_FRAMES: u32 = 256;

fn default_timestamp() -> AudioTimeStamp {
    let mut ts: AudioTimeStamp = unsafe { std::mem::zeroed() };
    ts.mHostTime = 0;
    ts.mFlags = kAudioTimeStampHostTimeValid;
    ts
}

#[test]
fn loopback_process_delivers_mic_audio() {
    let handle = unsafe { loopback_mixer_create(SAMPLE_RATE, BLOCK_FRAMES) };
    assert!(!handle.is_null(), "expected loopback mixer handle");

    let mut output = vec![0.0f32; (BLOCK_FRAMES as usize) * 2];
    let timestamp = default_timestamp();
    let mut audio_buffer = AudioBuffer {
        mNumberChannels: 2,
        mDataByteSize: (output.len() * std::mem::size_of::<f32>()) as u32,
        mData: output.as_mut_ptr() as *mut _,
    };
    let mut buffer_list = AudioBufferList {
        mNumberBuffers: 1,
        mBuffers: [audio_buffer],
    };
    let args = LoopbackRenderArgs {
        buffer_list: &mut buffer_list as *mut _,
        frame_count: BLOCK_FRAMES,
        timestamp: &timestamp as *const _,
    };

    let mut input = vec![0.0f32; (BLOCK_FRAMES as usize) * 2];
    for (i, frame) in input.chunks_exact_mut(2).enumerate() {
        let phase = i as f32 / SAMPLE_RATE as f32;
        let value = (phase * 440.0 * TAU).sin() * 0.5;
        frame[0] = value;
        frame[1] = value;
    }
    unsafe { loopback_mixer_submit_input(handle, input.as_ptr(), BLOCK_FRAMES) };

    let mut produced = false;
    for _ in 0..4 {
        output.fill(0.0);
        let status = unsafe { loopback_mixer_process(handle, &args) };
        assert_eq!(status, 0, "loopback_mixer_process returned {status}");
        if output.iter().any(|sample| sample.abs() > 1e-5) {
            produced = true;
            break;
        }
    }

    unsafe { loopback_mixer_destroy(handle) };
    assert!(produced, "expected non-silent output from mic submit path");
}

#[test]
fn loopback_node_source_push() {
    let handle = unsafe { loopback_mixer_create(SAMPLE_RATE, BLOCK_FRAMES) };
    assert!(!handle.is_null());
    assert!(unsafe { loopback_mixer_register_node_source(handle, 1, 4_096) });
    assert!(unsafe { loopback_mixer_set_node_gain(handle, 1, 1.0) });

    let mut output = vec![0.0f32; (BLOCK_FRAMES as usize) * 2];
    let timestamp = default_timestamp();
    let mut audio_buffer = AudioBuffer {
        mNumberChannels: 2,
        mDataByteSize: (output.len() * std::mem::size_of::<f32>()) as u32,
        mData: output.as_mut_ptr() as *mut _,
    };
    let mut buffer_list = AudioBufferList {
        mNumberBuffers: 1,
        mBuffers: [audio_buffer],
    };
    let args = LoopbackRenderArgs {
        buffer_list: &mut buffer_list as *mut _,
        frame_count: BLOCK_FRAMES,
        timestamp: &timestamp as *const _,
    };

    let mut node_input = vec![0.0f32; (BLOCK_FRAMES as usize) * 2];
    for (i, frame) in node_input.chunks_exact_mut(2).enumerate() {
        let phase = i as f32 / SAMPLE_RATE as f32;
        let value = (phase * 660.0 * TAU).sin() * 0.25;
        frame[0] = value;
        frame[1] = value;
    }

    let timestamp_ns = device_kit_monotonic_time_ns();
    assert!(unsafe {
        loopback_mixer_push_node_frames(handle, 1, node_input.as_ptr(), BLOCK_FRAMES, timestamp_ns)
    });

    let mut produced = false;
    for _ in 0..6 {
        output.fill(0.0);
        let status = unsafe { loopback_mixer_process(handle, &args) };
        assert_eq!(status, 0);
        if output.iter().any(|sample| sample.abs() > 1e-5) {
            produced = true;
            break;
        }
        let timestamp_ns = device_kit_monotonic_time_ns();
        assert!(unsafe {
            loopback_mixer_push_node_frames(
                handle,
                1,
                node_input.as_ptr(),
                BLOCK_FRAMES,
                timestamp_ns,
            )
        });
    }

    unsafe { loopback_mixer_destroy(handle) };
    assert!(produced, "expected non-silent output from node source");
}
