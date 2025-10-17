#![deny(missing_docs)]
#![allow(clippy::too_many_arguments)]

//! Real-time safe stereo audio mixer core with shared-memory ring buffers.
//!
//! The `Mixer` owns per-source [`SharedRingBuffer`](ring::SharedRingBuffer) instances that receive
//! interleaved `f32` PCM frames from Swift or Node bridges. The mixer performs lock-free, allocation
//! free processing in the audio callback, supporting per-source gain/mute, latency compensation,
//! and fractional resampling driven by device clock feedback.

use std::collections::{HashMap, VecDeque};
use std::convert::TryFrom;
use std::ffi::{CString, c_void};
use std::os::raw::c_char;
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, Once};

use dasp_frame::Frame;
use dasp_frame::Stereo;

use once_cell::sync::Lazy;
use parking_lot::{Mutex, RwLock};
use tracing::debug;

use coreaudio_sys::{
    AudioBufferList, AudioTimeStamp, OSStatus, kAudioHardwareUnspecifiedError,
    kAudioTimeStampHostTimeValid,
};

use crate::latency::{LatencyProbe, LatencyReport};
use crate::ring::{SharedRingBuffer, host_time_to_ns, monotonic_timestamp_ns};

/// Developer-facing control and TUI support.
pub mod control;
pub mod latency;
pub mod ring;

#[cfg(test)]
mod tests;

const MIX_CHANNELS: usize = 2;

static LOG_BUFFER: Lazy<Mutex<VecDeque<String>>> =
    Lazy::new(|| Mutex::new(VecDeque::with_capacity(64)));
static LOG_CACHE: Lazy<Mutex<Option<CString>>> = Lazy::new(|| Mutex::new(None));
static TRACING_INIT: Once = Once::new();
static DRIVER_RUNNING: AtomicBool = AtomicBool::new(false);
static ENGINE_RUNNING: AtomicBool = AtomicBool::new(false);

fn init_tracing() {
    TRACING_INIT.call_once(|| {
        let _ = tracing_subscriber::fmt::try_init();
    });
}

fn push_log(line: impl Into<String>) {
    let mut guard = LOG_BUFFER.lock();
    guard.push_back(line.into());
    while guard.len() > 256 {
        guard.pop_front();
    }
}

/// Interleaved floating-point audio buffer shared across the FFI boundary.
#[repr(C)]
pub struct AudioBuffer {
    /// Pointer to mutable interleaved `f32` frames.
    pub data: *mut f32,
    /// Number of frames (not samples) available at `data`.
    pub frames: u32,
    /// Channel count for `data`. Currently must be 2.
    pub channels: u32,
    /// Host-provided timestamp in nanoseconds for the first frame in the buffer.
    pub timestamp_ns: u64,
}

unsafe impl Send for AudioBuffer {}
unsafe impl Sync for AudioBuffer {}

/// Handle referencing a registered mixer source.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceHandle {
    id: u32,
}

impl SourceHandle {
    fn new(id: u32) -> Self {
        Self { id }
    }
}

/// Error enumeration surfaced across the public API.
#[derive(thiserror::Error, Debug)]
pub enum MixerError {
    /// Mixer pointer passed over FFI was null.
    #[error("null mixer pointer")]
    NullMixer,
    /// Source handle referenced an unknown source.
    #[error("unknown source id: {0}")]
    UnknownSource(u32),
    /// Requested channel configuration is unsupported.
    #[error("unsupported channel count {0}, only stereo is supported")]
    UnsupportedChannels(u32),
}

/// Resampler state with drift tracking.
struct ResamplerState {
    ratio_bits: std::sync::atomic::AtomicU32,
    phase: f32,
}

impl ResamplerState {
    fn new() -> Self {
        Self {
            ratio_bits: std::sync::atomic::AtomicU32::new(1.0f32.to_bits()),
            phase: 0.0,
        }
    }

    fn ratio(&self) -> f32 {
        f32::from_bits(self.ratio_bits.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn set_ratio(&self, ratio: f32) {
        self.ratio_bits
            .store(ratio.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
}

/// Clock feedback integrator maintains a smoothed drift estimate.
struct ClockState {
    last_device_ts: Option<u64>,
    last_source_ts: Option<u64>,
    smoothed_ratio: f32,
}

impl ClockState {
    fn new() -> Self {
        Self {
            last_device_ts: None,
            last_source_ts: None,
            smoothed_ratio: 1.0,
        }
    }

    fn submit_feedback(&mut self, device_ts: u64, source_ts: u64) -> Option<f32> {
        match (self.last_device_ts, self.last_source_ts) {
            (Some(prev_device), Some(prev_source))
                if device_ts > prev_device && source_ts > prev_source =>
            {
                let device_delta = (device_ts - prev_device) as f64;
                let source_delta = (source_ts - prev_source) as f64;
                if device_delta > 0.0 && source_delta > 0.0 {
                    let raw_ratio = (source_delta / device_delta) as f32;
                    let clamped = raw_ratio.clamp(0.98, 1.02);
                    // Critically damped first-order IIR smoother.
                    const ALPHA: f32 = 0.05;
                    self.smoothed_ratio =
                        self.smoothed_ratio + ALPHA * (clamped - self.smoothed_ratio);
                    self.last_device_ts = Some(device_ts);
                    self.last_source_ts = Some(source_ts);
                    return Some(self.smoothed_ratio);
                }
            }
            _ => {}
        }

        self.last_device_ts = Some(device_ts);
        self.last_source_ts = Some(source_ts);
        None
    }

    fn drift_ppm(&self) -> f32 {
        (self.smoothed_ratio - 1.0) * 1_000_000.0
    }
}

/// Delay line storing decoded frames to satisfy positive latency offsets.
struct DelayLine {
    buffer: Vec<Stereo<f32>>,
    capacity: usize,
    read_idx: usize,
    write_idx: usize,
    len: usize,
    target_delay: usize,
}

impl DelayLine {
    fn new(capacity: usize) -> Self {
        let capacity = capacity.max(32);
        Self {
            buffer: vec![Stereo::EQUILIBRIUM; capacity],
            capacity,
            read_idx: 0,
            write_idx: 0,
            len: 0,
            target_delay: 0,
        }
    }

    fn set_target(&mut self, frames: usize) {
        self.target_delay = frames.min(self.capacity.saturating_sub(1));
    }

    fn drop_frames(&mut self, frames: usize) -> usize {
        let drop = frames.min(self.len);
        for _ in 0..drop {
            self.pop_internal();
        }
        drop
    }

    fn pop_internal(&mut self) -> Option<Stereo<f32>> {
        if self.len == 0 {
            return None;
        }
        let frame = self.buffer[self.read_idx];
        self.read_idx = (self.read_idx + 1) % self.capacity;
        self.len -= 1;
        Some(frame)
    }

    fn process_frame(&mut self, frame: Stereo<f32>) -> Stereo<f32> {
        self.buffer[self.write_idx] = frame;
        self.write_idx = (self.write_idx + 1) % self.capacity;
        if self.len < self.capacity {
            self.len += 1;
        } else {
            self.read_idx = (self.read_idx + 1) % self.capacity;
        }

        if self.len > self.target_delay {
            self.pop_internal().unwrap_or(Stereo::EQUILIBRIUM)
        } else {
            Stereo::EQUILIBRIUM
        }
    }
}

/// Mixer source entry.
struct Source {
    handle: SourceHandle,
    ring: Arc<SharedRingBuffer>,
    gain: std::sync::atomic::AtomicU32,
    mute: std::sync::atomic::AtomicBool,
    latency_frames: std::sync::atomic::AtomicI64,
    current_latency_setting: i64,
    advance_deficit: usize,
    delay_line: DelayLine,
    resampler: ResamplerState,
    clock: ClockState,
    scratch: Vec<f32>,
    prev_frame: Stereo<f32>,
}

impl Source {
    fn new(handle: SourceHandle, ring: Arc<SharedRingBuffer>, max_block_frames: usize) -> Self {
        let scratch_samples = max_block_frames * MIX_CHANNELS * 4;
        Self {
            handle,
            ring,
            gain: std::sync::atomic::AtomicU32::new(1.0f32.to_bits()),
            mute: std::sync::atomic::AtomicBool::new(false),
            latency_frames: std::sync::atomic::AtomicI64::new(0),
            current_latency_setting: 0,
            advance_deficit: 0,
            delay_line: DelayLine::new(max_block_frames * 8),
            resampler: ResamplerState::new(),
            clock: ClockState::new(),
            scratch: vec![0.0; scratch_samples],
            prev_frame: Stereo::EQUILIBRIUM,
        }
    }

    fn set_gain(&self, gain: f32) {
        self.gain
            .store(gain.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn set_mute(&self, mute: bool) {
        self.mute.store(mute, std::sync::atomic::Ordering::Relaxed);
    }

    fn is_muted(&self) -> bool {
        self.mute.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn set_latency(&self, frames: i64) {
        self.latency_frames
            .store(frames, std::sync::atomic::Ordering::Relaxed);
    }

    fn update_latency_state(&mut self) {
        let desired = self
            .latency_frames
            .load(std::sync::atomic::Ordering::Relaxed);
        if desired == self.current_latency_setting {
            return;
        }

        if desired >= 0 {
            self.delay_line.set_target(desired as usize);
            self.advance_deficit = 0;
        } else {
            let deficit = desired.unsigned_abs() as usize;
            self.delay_line.set_target(0);
            let dropped = self.delay_line.drop_frames(deficit);
            self.advance_deficit = deficit.saturating_sub(dropped);
        }
        self.current_latency_setting = desired;
    }

    fn set_resample_ratio(&self, ratio: f32) {
        self.resampler.set_ratio(ratio);
    }

    fn apply_clock_feedback(&mut self, device_ts: u64, source_ts: u64) {
        if let Some(smoothed) = self.clock.submit_feedback(device_ts, source_ts) {
            self.set_resample_ratio(smoothed);
        }
    }

    fn write_from_slice(&self, data: &[f32], timestamp_ns: Option<u64>) -> usize {
        self.ring.push(data, timestamp_ns)
    }

    fn mix_into(&mut self, output: &mut [f32], frames: usize) {
        if self.is_muted() {
            return;
        }
        self.update_latency_state();

        if self.advance_deficit > 0 {
            let dropped = self.ring.discard(self.advance_deficit);
            self.advance_deficit = self.advance_deficit.saturating_sub(dropped);
        }

        let ratio = self.resampler.ratio().clamp(0.95, 1.05);
        let expected_input = ((frames as f32) * ratio).ceil() as usize + 2;
        let frame_samples = MIX_CHANNELS;
        let scratch_needed = expected_input * frame_samples;
        if scratch_needed > self.scratch.len() {
            // Real-time path must not reallocate; clamp size.
            return;
        }

        // Seed first frame with previous value for smooth interpolation.
        let mut total_input_frames = 1usize;
        self.scratch[0] = self.prev_frame[0];
        self.scratch[1] = self.prev_frame[1];

        let to_read_frames = expected_input.saturating_sub(1);
        let read_samples = to_read_frames * frame_samples;
        let read = self
            .ring
            .pop(&mut self.scratch[frame_samples..frame_samples + read_samples]);
        total_input_frames += read;

        let gain = self.gain();

        if total_input_frames < 2 {
            for frame_index in 0..frames {
                let delayed = self.delay_line.process_frame(Stereo::EQUILIBRIUM);
                let base = frame_index * frame_samples;
                output[base] += delayed[0] * gain;
                output[base + 1] += delayed[1] * gain;
            }
            return;
        }

        let mut produced_frames = 0usize;
        let mut input_cursor = 0usize;
        let mut phase = self.resampler.phase;
        let last_available = total_input_frames.saturating_sub(1);

        while produced_frames < frames {
            let frame = if input_cursor >= last_available {
                Stereo::EQUILIBRIUM
            } else {
                let base_idx = input_cursor;
                let next_idx = (input_cursor + 1).min(last_available);
                let frame_a = read_interleaved(&self.scratch, base_idx);
                let frame_b = read_interleaved(&self.scratch, next_idx);
                let t = phase;
                [
                    frame_a[0] + (frame_b[0] - frame_a[0]) * t,
                    frame_a[1] + (frame_b[1] - frame_a[1]) * t,
                ]
            };

            phase += ratio;
            let advance = phase.floor() as usize;
            if advance > 0 {
                phase -= advance as f32;
                input_cursor = (input_cursor + advance).min(last_available);
            }

            let delayed = self.delay_line.process_frame(frame);
            let base = produced_frames * frame_samples;
            output[base] += delayed[0] * gain;
            output[base + 1] += delayed[1] * gain;
            produced_frames += 1;
        }

        self.resampler.phase = phase;
        self.prev_frame = read_interleaved(&self.scratch, last_available);
    }

    fn buffer_fill_ratio(&self) -> f32 {
        let capacity = self.ring.capacity_frames();
        if capacity == 0 {
            return 0.0;
        }
        self.ring.available_read() as f32 / capacity as f32
    }

    fn latency_frames(&self) -> i64 {
        self.current_latency_setting
    }

    fn drift_ppm(&self) -> f32 {
        self.clock.drift_ppm()
    }

    fn rms_estimate(&self) -> f32 {
        let left = self.prev_frame[0];
        let right = self.prev_frame[1];
        ((left * left + right * right) * 0.5).sqrt()
    }

    fn gain_linear(&self) -> f32 {
        self.gain()
    }
}

fn read_interleaved(buffer: &[f32], frame_index: usize) -> Stereo<f32> {
    let base = frame_index * MIX_CHANNELS;
    [buffer[base], buffer[base + 1]]
}

/// Primary mixer struct orchestrating all decoding and mixing.
pub struct Mixer {
    sample_rate: u32,
    max_block_frames: usize,
    sources: Vec<Source>,
    next_source_id: u32,
    latency_probe: LatencyProbe,
}

/// Per-source diagnostics exposed to developer tooling.
#[derive(Clone, Debug)]
pub struct SourceStatus {
    /// Numeric identifier of the source.
    pub id: u32,
    /// Friendly name inferred from registration.
    pub name: String,
    /// Linear gain applied to the source.
    pub gain_linear: f32,
    /// Gain expressed in decibels for presentation.
    pub gain_db: f32,
    /// Whether the source is muted.
    pub muted: bool,
    /// Configured latency in frames (positive adds delay, negative advances).
    pub latency_frames: i64,
    /// Estimated buffer utilisation percentage for queued audio.
    pub buffer_fill: f32,
    /// Estimated RMS level (0-1).
    pub rms: f32,
    /// Clock drift estimate in parts per million.
    pub drift_ppm: f32,
}

/// Aggregated mixer status snapshot used by control surfaces.
#[derive(Clone, Debug)]
pub struct MixerStatus {
    /// Current sample rate in Hertz.
    pub sample_rate: u32,
    /// Maximum block size in frames requested by the host.
    pub buffer_frames: usize,
    /// Effective render latency in milliseconds based on buffer size.
    pub latency_ms: f32,
    /// Approximate mixer CPU utilisation (0–1 range).
    pub cpu_usage: f32,
    /// Average queued buffer fill across active sources (0–1).
    pub buffer_fill: f32,
    /// Average drift estimate in parts per million.
    pub drift_ppm: f32,
    /// Per-source diagnostics.
    pub sources: Vec<SourceStatus>,
}

/// Telemetry snapshot used to report input/output RMS levels across the FFI boundary.
#[repr(C)]
pub struct LoopbackLevels {
    /// Latest input RMS levels (up to 8 channels).
    pub inputs: [f32; 8],
    /// Latest output RMS levels (up to 8 channels).
    pub outputs: [f32; 8],
    /// Number of valid entries in `inputs`.
    pub input_count: u32,
    /// Number of valid entries in `outputs`.
    pub output_count: u32,
}

impl Mixer {
    /// Construct a new mixer.
    pub fn new(sample_rate: u32, max_block_frames: usize) -> Self {
        Self {
            sample_rate,
            max_block_frames,
            sources: Vec::new(),
            next_source_id: 1,
            latency_probe: LatencyProbe::new(sample_rate, 440.0, sample_rate as usize / 10),
        }
    }

    /// Register a new source using a locally managed shared ring buffer.
    pub fn add_source(&mut self, capacity_frames: usize) -> (SourceHandle, Arc<SharedRingBuffer>) {
        let handle = SourceHandle::new(self.next_source_id);
        self.next_source_id += 1;
        let ring = Arc::new(SharedRingBuffer::new_local(capacity_frames, MIX_CHANNELS));
        let source = Source::new(handle, ring.clone(), self.max_block_frames);
        self.sources.push(source);
        (handle, ring)
    }

    /// Register a source backed by an externally provided shared memory ring.
    pub fn add_external_source(&mut self, ring: Arc<SharedRingBuffer>) -> SourceHandle {
        let handle = SourceHandle::new(self.next_source_id);
        self.next_source_id += 1;
        let source = Source::new(handle, ring, self.max_block_frames);
        self.sources.push(source);
        handle
    }

    fn source_mut(&mut self, handle: SourceHandle) -> Option<&mut Source> {
        self.sources.iter_mut().find(|s| s.handle == handle)
    }

    fn source(&self, handle: SourceHandle) -> Option<&Source> {
        self.sources.iter().find(|s| s.handle == handle)
    }

    /// Mix into the provided output buffer. Returns frames rendered.
    pub fn process(&mut self, buffer: &mut AudioBuffer) -> Result<usize, MixerError> {
        if buffer.channels != MIX_CHANNELS as u32 {
            return Err(MixerError::UnsupportedChannels(buffer.channels));
        }
        let frames = buffer.frames as usize;
        if frames == 0 {
            return Ok(0);
        }
        let output = unsafe { std::slice::from_raw_parts_mut(buffer.data, frames * MIX_CHANNELS) };
        output.fill(0.0);

        for source in &mut self.sources {
            source.mix_into(output, frames);
        }
        Ok(frames)
    }

    /// Convenience method to write PCM frames into a source's ring.
    pub fn write_source(
        &mut self,
        handle: SourceHandle,
        frames: &[f32],
        timestamp_ns: Option<u64>,
    ) -> Result<usize, MixerError> {
        let source = self
            .source(handle)
            .ok_or(MixerError::UnknownSource(handle.id))?;
        Ok(source.write_from_slice(frames, timestamp_ns))
    }

    /// Adjust per-source gain.
    pub fn set_gain(&mut self, handle: SourceHandle, gain: f32) -> Result<(), MixerError> {
        let source = self
            .source(handle)
            .ok_or(MixerError::UnknownSource(handle.id))?;
        source.set_gain(gain);
        Ok(())
    }

    /// Toggle mute for a source.
    pub fn set_mute(&mut self, handle: SourceHandle, mute: bool) -> Result<(), MixerError> {
        let source = self
            .source(handle)
            .ok_or(MixerError::UnknownSource(handle.id))?;
        source.set_mute(mute);
        Ok(())
    }

    /// Configure latency compensation in frames for a source. Positive delays audio, negative advances.
    pub fn set_latency(&mut self, handle: SourceHandle, frames: i32) -> Result<(), MixerError> {
        let source = self
            .source(handle)
            .ok_or(MixerError::UnknownSource(handle.id))?;
        source.set_latency(frames as i64);
        Ok(())
    }

    /// Provide device clock feedback for drift correction.
    pub fn submit_clock_feedback(
        &mut self,
        handle: SourceHandle,
        device_timestamp_ns: u64,
        source_timestamp_ns: u64,
    ) -> Result<(), MixerError> {
        let source = self
            .source_mut(handle)
            .ok_or(MixerError::UnknownSource(handle.id))?;
        source.apply_clock_feedback(device_timestamp_ns, source_timestamp_ns);
        Ok(())
    }

    /// Fetch the latency probe for testing.
    pub fn latency_probe(&self) -> &LatencyProbe {
        &self.latency_probe
    }

    /// Acquire latency metrics against recorded audio.
    pub fn measure_latency(&self, recorded: &[f32]) -> LatencyReport {
        self.latency_probe.measure(recorded)
    }

    fn collect_status(&self, mic_handle: SourceHandle) -> (Vec<SourceStatus>, f32, f32) {
        let mut total_fill = 0.0f32;
        let mut total_drift = 0.0f32;
        let mut statuses = Vec::with_capacity(self.sources.len());

        for source in &self.sources {
            let name = if source.handle == mic_handle {
                "Microphone".to_string()
            } else {
                format!("Source #{}", source.handle.id)
            };

            let gain_linear = source.gain_linear();
            let gain_db = if gain_linear > 0.0 {
                20.0 * gain_linear.log10()
            } else {
                f32::NEG_INFINITY
            };

            let buffer_fill = source.buffer_fill_ratio().clamp(0.0, 1.0);
            let drift_ppm = source.drift_ppm();
            total_fill += buffer_fill;
            total_drift += drift_ppm.abs();

            statuses.push(SourceStatus {
                id: source.handle.id,
                name,
                gain_linear,
                gain_db,
                muted: source.is_muted(),
                latency_frames: source.latency_frames(),
                buffer_fill,
                rms: source.rms_estimate().clamp(0.0, 1.0),
                drift_ppm,
            });
        }

        let avg_fill = if statuses.is_empty() {
            0.0
        } else {
            total_fill / statuses.len() as f32
        };
        let avg_drift = if statuses.is_empty() {
            0.0
        } else {
            total_drift / statuses.len() as f32
        };

        (statuses, avg_fill, avg_drift)
    }
}

/// FFI render arguments matching the C bridge header layout.
#[repr(C)]
pub struct LoopbackRenderArgs {
    /// Pointer to an `AudioBufferList` containing interleaved stereo data.
    pub buffer_list: *mut AudioBufferList,
    /// Number of frames referenced in the render quantum.
    pub frame_count: u32,
    /// Timestamp provided by Core Audio for the render block.
    pub timestamp: *const AudioTimeStamp,
}

#[derive(Clone)]
struct NodeSourceEntry {
    handle: SourceHandle,
    ring: Arc<SharedRingBuffer>,
}

/// Exposed mixer wrapper bridging the CoreAudio loopback driver with the Rust core engine.
pub struct LoopbackMixerFfi {
    mixer: Mixer,
    mic_handle: SourceHandle,
    node_sources: RwLock<HashMap<u32, NodeSourceEntry>>,
}

impl LoopbackMixerFfi {
    fn new(sample_rate: f64, max_frames: u32) -> Option<Self> {
        let sr = sample_rate.round().clamp(8_000.0, 192_000.0) as u32;
        let mut mixer = Mixer::new(sr, max_frames as usize);
        let (mic_handle, _ring) = mixer.add_source((max_frames.max(256)) as usize * 4);
        Some(Self {
            mixer,
            mic_handle,
            node_sources: RwLock::new(HashMap::new()),
        })
    }

    fn timestamp_ns(&self, timestamp: *const AudioTimeStamp) -> u64 {
        if timestamp.is_null() {
            return monotonic_timestamp_ns();
        }
        let ts = unsafe { &*timestamp };
        if (ts.mFlags & kAudioTimeStampHostTimeValid) != 0 {
            let ns = host_time_to_ns(ts.mHostTime);
            if ns == 0 {
                monotonic_timestamp_ns()
            } else {
                ns
            }
        } else {
            monotonic_timestamp_ns()
        }
    }

    fn process(&mut self, args: &LoopbackRenderArgs) -> Result<(), MixerError> {
        if args.frame_count == 0 {
            return Ok(());
        }
        let buffer_list = unsafe { args.buffer_list.as_mut().ok_or(MixerError::NullMixer)? };
        if buffer_list.mNumberBuffers == 0 {
            return Err(MixerError::UnsupportedChannels(0));
        }
        let buffer = unsafe { &mut *buffer_list.mBuffers.as_mut_ptr() };
        if buffer.mNumberChannels != MIX_CHANNELS as u32 {
            return Err(MixerError::UnsupportedChannels(buffer.mNumberChannels));
        }
        if buffer.mData.is_null() {
            return Err(MixerError::NullMixer);
        }

        let frames = args.frame_count;
        let samples = frames as usize * MIX_CHANNELS;
        let slice = unsafe { slice::from_raw_parts_mut(buffer.mData as *mut f32, samples) };
        let timestamp_ns = self.timestamp_ns(args.timestamp);
        let mut audio_buffer = AudioBuffer {
            data: slice.as_mut_ptr(),
            frames,
            channels: buffer.mNumberChannels,
            timestamp_ns,
        };
        slice.fill(0.0);
        self.mixer.process(&mut audio_buffer).map(|_| ())
    }

    fn submit_input(&mut self, data: *const f32, frames: u32) {
        if data.is_null() || frames == 0 {
            return;
        }
        let samples = frames as usize * MIX_CHANNELS;
        let slice = unsafe { slice::from_raw_parts(data, samples) };
        let _ = self
            .mixer
            .write_source(self.mic_handle, slice, Some(monotonic_timestamp_ns()));
    }

    fn register_node_source(&mut self, source_index: u32, capacity_frames: usize) -> bool {
        if self.node_sources.read().contains_key(&source_index) {
            return true;
        }
        let (handle, ring) = self.mixer.add_source(capacity_frames);
        let entry = NodeSourceEntry { handle, ring };
        self.node_sources.write().insert(source_index, entry);
        true
    }

    fn node_entry(&self, source_index: u32) -> Option<NodeSourceEntry> {
        self.node_sources.read().get(&source_index).cloned()
    }

    fn push_node_frames(&self, source_index: u32, data: &[f32], timestamp_ns: u64) -> bool {
        let Some(entry) = self.node_entry(source_index) else {
            return false;
        };
        if data.is_empty() {
            return true;
        }
        if data.len() % MIX_CHANNELS != 0 {
            return false;
        }
        let frames = data.len() / MIX_CHANNELS;
        let written = entry.ring.push(data, Some(timestamp_ns));
        if written < frames {
            let drop_frames = frames - written;
            entry.ring.discard(drop_frames);
            let start = written * MIX_CHANNELS;
            let _ = entry.ring.push(&data[start..], Some(timestamp_ns));
        }
        true
    }

    fn set_gain(&mut self, source_index: u32, gain: f32) -> bool {
        if source_index == 0 {
            let _ = self.mixer.set_gain(self.mic_handle, gain);
            return true;
        }
        if let Some(entry) = self.node_entry(source_index) {
            let _ = self.mixer.set_gain(entry.handle, gain);
            true
        } else {
            false
        }
    }

    fn set_mute(&mut self, source_index: u32, mute: bool) -> bool {
        if source_index == 0 {
            let _ = self.mixer.set_mute(self.mic_handle, mute);
            return true;
        }
        if let Some(entry) = self.node_entry(source_index) {
            let _ = self.mixer.set_mute(entry.handle, mute);
            true
        } else {
            false
        }
    }

    fn status(&self) -> MixerStatus {
        let (sources, avg_fill, avg_drift) = self.mixer.collect_status(self.mic_handle);
        let sample_rate = self.mixer.sample_rate;
        let buffer_frames = self.mixer.max_block_frames;
        let latency_ms = if sample_rate == 0 {
            0.0
        } else {
            (buffer_frames as f32 / sample_rate as f32) * 1_000.0
        };

        MixerStatus {
            sample_rate,
            buffer_frames,
            latency_ms,
            cpu_usage: 0.0,
            buffer_fill: avg_fill,
            drift_ppm: avg_drift,
            sources,
        }
    }
}

static LOOPBACK_GLOBAL: AtomicPtr<LoopbackMixerFfi> = AtomicPtr::new(ptr::null_mut());
static SOURCE_ENABLE_STATE: Lazy<Mutex<HashMap<u32, bool>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Create a new mixer instance.
#[unsafe(no_mangle)]
pub extern "C" fn device_kit_mixer_new(sample_rate: u32, max_block_frames: u32) -> *mut Mixer {
    Box::into_raw(Box::new(Mixer::new(sample_rate, max_block_frames as usize)))
}

/// Create a loopback mixer handle suitable for DriverKit.
#[unsafe(no_mangle)]
pub extern "C" fn loopback_mixer_create(
    sample_rate: f64,
    max_frames: u32,
) -> *mut LoopbackMixerFfi {
    init_tracing();
    let Some(mixer) = LoopbackMixerFfi::new(sample_rate, max_frames) else {
        return ptr::null_mut();
    };
    let raw = Box::into_raw(Box::new(mixer));
    LOOPBACK_GLOBAL.store(raw, Ordering::SeqCst);
    raw
}

/// Free an allocated mixer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_kit_mixer_free(ptr: *mut Mixer) {
    if !ptr.is_null() {
        unsafe {
            drop(Box::from_raw(ptr));
        }
    }
}

/// Destroy a loopback mixer handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loopback_mixer_destroy(handle: *mut LoopbackMixerFfi) {
    if !handle.is_null() {
        unsafe {
            let stored = LOOPBACK_GLOBAL.load(Ordering::SeqCst);
            if stored == handle {
                LOOPBACK_GLOBAL.store(ptr::null_mut(), Ordering::SeqCst);
            }
            drop(Box::from_raw(handle));
        }
    }
}

/// Process a render quantum for the loopback device.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loopback_mixer_process(
    handle: *mut LoopbackMixerFfi,
    args: *const LoopbackRenderArgs,
) -> OSStatus {
    if handle.is_null() || args.is_null() {
        return kAudioHardwareUnspecifiedError.try_into().unwrap();
    }
    let (result, frames) = unsafe {
        let mixer = &mut *handle;
        let args = &*args;
        let frames = args.frame_count;
        (mixer.process(args), frames)
    };
    debug!(frames = frames, "process_audio");
    match &result {
        Ok(_) => push_log(format!("process_audio ok frames={frames}")),
        Err(err) => push_log(format!("process_audio error frames={frames}: {err}")),
    }
    translate_status(result)
}

fn translate_status(result: Result<(), MixerError>) -> OSStatus {
    match result {
        Ok(()) => 0,
        Err(_) => kAudioHardwareUnspecifiedError.try_into().unwrap(),
    }
}

/// Submit microphone input frames into the loopback mixer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loopback_mixer_submit_input(
    handle: *mut LoopbackMixerFfi,
    data: *const f32,
    frames: u32,
) {
    if handle.is_null() {
        return;
    }
    unsafe {
        let mixer = &mut *handle;
        mixer.submit_input(data, frames);
    }
}

/// Adjust per-source gain on the loopback mixer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loopback_mixer_set_gain(
    handle: *mut LoopbackMixerFfi,
    source_index: u32,
    gain: f32,
) {
    if handle.is_null() {
        return;
    }
    unsafe {
        let mixer = &mut *handle;
        let _ = mixer.set_gain(source_index, gain);
    }
}

/// Adjust per-source mute state on the loopback mixer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loopback_mixer_set_mute(
    handle: *mut LoopbackMixerFfi,
    source_index: u32,
    mute: bool,
) {
    if handle.is_null() {
        return;
    }
    unsafe {
        let mixer = &mut *handle;
        let _ = mixer.set_mute(source_index, mute);
    }
}

/// Register a node-managed source that can be fed from NodeJS.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loopback_mixer_register_node_source(
    handle: *mut LoopbackMixerFfi,
    source_index: u32,
    capacity_frames: u32,
) -> bool {
    if handle.is_null() {
        return false;
    }
    unsafe {
        let mixer = &mut *handle;
        mixer.register_node_source(source_index, capacity_frames as usize)
    }
}

/// Push PCM frames supplied by NodeJS into the async ring buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loopback_mixer_push_node_frames(
    handle: *mut LoopbackMixerFfi,
    source_index: u32,
    data: *const f32,
    frames: u32,
    timestamp_ns: u64,
) -> bool {
    if handle.is_null() || data.is_null() || frames == 0 {
        return false;
    }
    unsafe {
        let mixer = &*handle;
        let samples = frames as usize * MIX_CHANNELS;
        let slice = slice::from_raw_parts(data, samples);
        mixer.push_node_frames(source_index, slice, timestamp_ns)
    }
}

/// Update gain for a NodeJS-driven source.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loopback_mixer_set_node_gain(
    handle: *mut LoopbackMixerFfi,
    source_index: u32,
    gain: f32,
) -> bool {
    if handle.is_null() {
        return false;
    }
    unsafe {
        let mixer = &mut *handle;
        mixer.set_gain(source_index, gain)
    }
}

/// Update mute state for a NodeJS-driven source.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn loopback_mixer_set_node_mute(
    handle: *mut LoopbackMixerFfi,
    source_index: u32,
    mute: bool,
) -> bool {
    if handle.is_null() {
        return false;
    }
    unsafe {
        let mixer = &mut *handle;
        mixer.set_mute(source_index, mute)
    }
}

/// Fetch the currently active loopback mixer handle, if any.
#[unsafe(no_mangle)]
pub extern "C" fn loopback_mixer_global_handle() -> *mut LoopbackMixerFfi {
    LOOPBACK_GLOBAL.load(Ordering::SeqCst)
}

/// Retrieve the current mixer status if a mixer is active.
pub fn get_mixer_status() -> Option<MixerStatus> {
    let handle = loopback_mixer_global_handle();
    if handle.is_null() {
        return None;
    }
    unsafe { Some((&*handle).status()) }
}

/// Set source gain expressed in decibels. Returns `false` if no mixer is active.
pub fn set_source_gain_db(source_id: u32, gain_db: f32) -> bool {
    let handle = loopback_mixer_global_handle();
    if handle.is_null() {
        return false;
    }
    let amplitude = if gain_db <= -120.0 {
        0.0
    } else {
        10f32.powf(gain_db / 20.0)
    };
    unsafe {
        loopback_mixer_set_gain(handle, source_id, amplitude);
    }
    true
}

/// Set the mute state of a mixer source. Returns `false` if no mixer is active.
pub fn set_source_mute(source_id: u32, muted: bool) -> bool {
    let handle = loopback_mixer_global_handle();
    if handle.is_null() {
        return false;
    }
    unsafe {
        loopback_mixer_set_mute(handle, source_id, muted);
    }
    true
}

#[unsafe(no_mangle)]
/// Populate a `LoopbackLevels` struct with the latest RMS measurements.
pub extern "C" fn device_kit_get_levels(levels_out: *mut LoopbackLevels) -> bool {
    if levels_out.is_null() {
        return false;
    }
    let mut levels = LoopbackLevels {
        inputs: [0.0; 8],
        outputs: [0.0; 8],
        input_count: 0,
        output_count: 0,
    };

    if let Some(status) = get_mixer_status() {
        for (idx, src) in status.sources.iter().enumerate().take(8) {
            levels.outputs[idx] = src.rms;
        }
        levels.output_count = status.sources.len().min(8) as u32;
    } else {
        unsafe {
            *levels_out = levels;
        }
        return false;
    }

    unsafe {
        *levels_out = levels;
    }
    true
}

#[unsafe(no_mangle)]
/// Return the current mixer sample rate in Hertz, or `0` if inactive.
pub extern "C" fn device_kit_current_sample_rate() -> f64 {
    get_mixer_status()
        .map(|status| status.sample_rate as f64)
        .unwrap_or(0.0)
}

#[unsafe(no_mangle)]
/// Return the mixer buffer size in frames, or `0` if inactive.
pub extern "C" fn device_kit_buffer_size_frames() -> u32 {
    get_mixer_status()
        .map(|status| status.buffer_frames as u32)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
/// Return the current mixer latency in milliseconds, or `0` if inactive.
pub extern "C" fn device_kit_latency_ms() -> f64 {
    get_mixer_status()
        .map(|status| status.latency_ms as f64)
        .unwrap_or(0.0)
}

#[unsafe(no_mangle)]
/// Start the loopback driver, initialising tracing on first activation.
pub extern "C" fn device_kit_start_driver() -> bool {
    init_tracing();
    let was_running = DRIVER_RUNNING.swap(true, Ordering::SeqCst);
    if !was_running {
        push_log("driver started");
    }
    true
}

#[unsafe(no_mangle)]
/// Stop the loopback driver and record the transition.
pub extern "C" fn device_kit_stop_driver() {
    if DRIVER_RUNNING.swap(false, Ordering::SeqCst) {
        push_log("driver stopped");
    }
}

#[unsafe(no_mangle)]
/// Start the loopback engine and mark it as active.
pub extern "C" fn device_kit_start_engine() -> bool {
    let was_running = ENGINE_RUNNING.swap(true, Ordering::SeqCst);
    if !was_running {
        push_log("engine started");
    }
    true
}

#[unsafe(no_mangle)]
/// Stop the loopback engine and record the transition.
pub extern "C" fn device_kit_stop_engine() {
    if ENGINE_RUNNING.swap(false, Ordering::SeqCst) {
        push_log("engine stopped");
    }
}

#[unsafe(no_mangle)]
/// Return the number of known mixer sources, or `0` if inactive.
pub extern "C" fn device_kit_source_count() -> u32 {
    get_mixer_status()
        .map(|status| status.sources.len() as u32)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
/// Check whether the specified source is currently enabled.
pub extern "C" fn device_kit_source_is_enabled(source_index: u32) -> bool {
    SOURCE_ENABLE_STATE
        .lock()
        .get(&source_index)
        .copied()
        .unwrap_or(true)
}

#[unsafe(no_mangle)]
/// Update the enabled state of the specified source and mute when disabled.
pub extern "C" fn device_kit_set_source_enabled(source_index: u32, enabled: bool) {
    SOURCE_ENABLE_STATE.lock().insert(source_index, enabled);
    let _ = set_source_mute(source_index, !enabled);
}

#[unsafe(no_mangle)]
/// Pop the next log entry produced by the mixer. Returns `NULL` when no logs remain.
pub extern "C" fn device_kit_pop_log() -> *const c_char {
    if let Some(message) = LOG_BUFFER.lock().pop_front() {
        let mut cache = LOG_CACHE.lock();
        *cache = Some(CString::new(message).unwrap_or_default());
        cache.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null())
    } else {
        ptr::null()
    }
}

/// Register a NodeJS source via the global mixer handle.
pub fn node_register_source(source_index: u32, capacity_frames: u32) -> bool {
    let handle = loopback_mixer_global_handle();
    if handle.is_null() {
        return false;
    }
    unsafe { loopback_mixer_register_node_source(handle, source_index, capacity_frames) }
}

/// Push PCM frames originating from NodeJS into the global mixer.
pub fn node_push_frames(source_index: u32, data: &[f32], timestamp_ns: u64) -> bool {
    if data.len() % MIX_CHANNELS != 0 {
        return false;
    }
    let frames = data.len() / MIX_CHANNELS;
    let Ok(frames_u32) = u32::try_from(frames) else {
        return false;
    };
    let handle = loopback_mixer_global_handle();
    if handle.is_null() {
        return false;
    }
    unsafe {
        loopback_mixer_push_node_frames(
            handle,
            source_index,
            data.as_ptr(),
            frames_u32,
            timestamp_ns,
        )
    }
}

/// Update gain for a NodeJS-managed source on the global mixer.
pub fn node_set_gain(source_index: u32, gain: f32) -> bool {
    let handle = loopback_mixer_global_handle();
    if handle.is_null() {
        return false;
    }
    unsafe { loopback_mixer_set_node_gain(handle, source_index, gain) }
}

/// Update mute state for a NodeJS-managed source on the global mixer.
pub fn node_set_mute(source_index: u32, mute: bool) -> bool {
    let handle = loopback_mixer_global_handle();
    if handle.is_null() {
        return false;
    }
    unsafe { loopback_mixer_set_node_mute(handle, source_index, mute) }
}

/// Add a new local ring buffer backed source and return its handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_kit_mixer_add_source(
    mixer: *mut Mixer,
    capacity_frames: u32,
    out_ring_header: *mut *mut c_void,
    out_ring_data: *mut *mut f32,
    out_ring_length: *mut usize,
) -> SourceHandle {
    if mixer.is_null() {
        return SourceHandle::new(0);
    }
    let mixer = unsafe { &mut *mixer };
    let (handle, ring) = mixer.add_source(capacity_frames as usize);
    if !out_ring_header.is_null() {
        unsafe {
            *out_ring_header = ring.raw_header_ptr() as *mut c_void;
        }
    }
    if !out_ring_data.is_null() {
        unsafe {
            *out_ring_data = ring.raw_data_ptr();
        }
    }
    if !out_ring_length.is_null() {
        unsafe {
            *out_ring_length = ring.capacity_samples();
        }
    }
    handle
}

/// Submit audio data into the specified source's ring.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_kit_source_write(
    mixer: *mut Mixer,
    handle: SourceHandle,
    data: *const f32,
    frames: u32,
    timestamp_ns: u64,
) -> usize {
    if mixer.is_null() || data.is_null() {
        return 0;
    }
    let mixer = unsafe { &mut *mixer };
    let slice = unsafe { std::slice::from_raw_parts(data, frames as usize * MIX_CHANNELS) };
    mixer
        .write_source(handle, slice, Some(timestamp_ns))
        .unwrap_or(0)
}

/// Mix into the provided buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_kit_mixer_process(
    mixer: *mut Mixer,
    buffer: *mut AudioBuffer,
) -> usize {
    if mixer.is_null() || buffer.is_null() {
        return 0;
    }
    let mixer = unsafe { &mut *mixer };
    let buffer = unsafe { &mut *buffer };
    match mixer.process(buffer) {
        Ok(frames) => frames,
        Err(_) => 0,
    }
}

/// Set per-source gain.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_kit_mixer_set_gain(
    mixer: *mut Mixer,
    handle: SourceHandle,
    gain: f32,
) {
    if mixer.is_null() {
        return;
    }
    let mixer = unsafe { &mut *mixer };
    let _ = mixer.set_gain(handle, gain);
}

/// Toggle per-source mute.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_kit_mixer_set_mute(
    mixer: *mut Mixer,
    handle: SourceHandle,
    mute: bool,
) {
    if mixer.is_null() {
        return;
    }
    let mixer = unsafe { &mut *mixer };
    let _ = mixer.set_mute(handle, mute);
}

/// Configure per-source latency compensation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_kit_mixer_set_latency(
    mixer: *mut Mixer,
    handle: SourceHandle,
    frames: i32,
) {
    if mixer.is_null() {
        return;
    }
    let mixer = unsafe { &mut *mixer };
    let _ = mixer.set_latency(handle, frames);
}

/// Submit device/source timestamp feedback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_kit_mixer_submit_clock(
    mixer: *mut Mixer,
    handle: SourceHandle,
    device_timestamp_ns: u64,
    source_timestamp_ns: u64,
) {
    if mixer.is_null() {
        return;
    }
    let mixer = unsafe { &mut *mixer };
    let _ = mixer.submit_clock_feedback(handle, device_timestamp_ns, source_timestamp_ns);
}

/// Populate an `AudioBuffer` for a latency probe test signal.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_kit_latency_probe_sine(
    mixer: *mut Mixer,
    buffer: *mut AudioBuffer,
    frequency_hz: f32,
) -> usize {
    if mixer.is_null() || buffer.is_null() {
        return 0;
    }
    let mixer = unsafe { &mut *mixer };
    let buffer = unsafe { &mut *buffer };
    if buffer.channels != MIX_CHANNELS as u32 {
        return 0;
    }
    let frames = buffer.frames as usize;
    let slice = unsafe { std::slice::from_raw_parts_mut(buffer.data, frames * MIX_CHANNELS) };
    mixer
        .latency_probe()
        .emit_sine(frequency_hz, slice)
        .min(frames)
}

/// Measure latency against a captured buffer. Returns offset in frames.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_kit_latency_measure(
    mixer: *mut Mixer,
    data: *const f32,
    frames: u32,
) -> i32 {
    if mixer.is_null() || data.is_null() {
        return -1;
    }
    let mixer = unsafe { &mut *mixer };
    let slice = unsafe { std::slice::from_raw_parts(data, frames as usize * MIX_CHANNELS) };
    mixer.measure_latency(slice).offset_frames as i32
}

/// Fetch the current monotonic timestamp in nanoseconds.
#[unsafe(no_mangle)]
pub extern "C" fn device_kit_monotonic_time_ns() -> u64 {
    monotonic_timestamp_ns()
}
