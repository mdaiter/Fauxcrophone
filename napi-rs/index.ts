/* eslint-disable @typescript-eslint/no-var-requires */
const binding = require('./index.node') as {
  register_source(channel: number, capacityFrames?: number): boolean;
  push_audio_frame(channel: number, pcm: Float32Array, timestamp?: number): boolean;
  set_source_gain(channel: number, gain: number): boolean;
  set_source_mute(channel: number, mute: boolean): boolean;
  monotonic_time_ns(): number;
};

export interface PushAudioFrameOptions {
  channel: number;
  pcm: Float32Array;
  /** Timestamp in nanoseconds. Defaults to the mixer monotonic clock. */
  timestampNs?: number;
}

export function registerSource(channel: number, capacityFrames = 4096): boolean {
  return binding.register_source(channel, capacityFrames);
}

export function pushAudioFrame(options: PushAudioFrameOptions): boolean {
  return binding.push_audio_frame(options.channel, options.pcm, options.timestampNs);
}

export function setSourceGain(channel: number, gain: number): boolean {
  return binding.set_source_gain(channel, gain);
}

export function setSourceMute(channel: number, mute: boolean): boolean {
  return binding.set_source_mute(channel, mute);
}

export function monotonicTimeNs(): number {
  return binding.monotonic_time_ns();
}
