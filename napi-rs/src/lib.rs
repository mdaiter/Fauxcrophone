#![deny(clippy::all)]

use napi::bindgen_prelude::{Error, Float32Array};
use napi_derive::napi;

const STEREO_CHANNELS: usize = 2;
const DEFAULT_RING_CAPACITY: u32 = 4_096;

fn ensure_capacity(capacity: Option<u32>) -> napi::Result<u32> {
    match capacity {
        Some(0) => Err(Error::from_reason(
            "capacityFrames must be greater than zero".to_string(),
        )),
        Some(value) => Ok(value),
        None => Ok(DEFAULT_RING_CAPACITY),
    }
}

#[napi]
pub fn register_source(channel: u32, capacity_frames: Option<u32>) -> napi::Result<bool> {
    let capacity = ensure_capacity(capacity_frames)?;
    Ok(device_kit::node_register_source(channel, capacity))
}

#[napi]
pub fn push_audio_frame(
    channel: u32,
    pcm: Float32Array,
    timestamp: Option<f64>,
) -> napi::Result<bool> {
    let slice = pcm.as_ref();
    if slice.len() % STEREO_CHANNELS != 0 {
        return Err(Error::from_reason(format!(
            "pcmBuffer length {} is not divisible by {} (stereo interleaved)",
            slice.len(),
            STEREO_CHANNELS
        )));
    }
    if slice.is_empty() {
        return Ok(true);
    }
    let timestamp_ns = timestamp
        .map(|value| value.max(0.0) as u64)
        .unwrap_or_else(|| device_kit::device_kit_monotonic_time_ns());
    Ok(device_kit::node_push_frames(channel, slice, timestamp_ns))
}

#[napi]
pub fn set_source_gain(channel: u32, gain: f64) -> napi::Result<bool> {
    Ok(device_kit::node_set_gain(channel, gain as f32))
}

#[napi]
pub fn set_source_mute(channel: u32, mute: bool) -> napi::Result<bool> {
    Ok(device_kit::node_set_mute(channel, mute))
}

#[napi]
pub fn monotonic_time_ns() -> napi::Result<f64> {
    Ok(device_kit::device_kit_monotonic_time_ns() as f64)
}
