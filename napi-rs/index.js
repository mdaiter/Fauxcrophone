/* eslint-disable @typescript-eslint/no-var-requires */
const binding = require('./index.node');

function registerSource(channel, capacityFrames = 4096) {
  return binding.register_source(channel, capacityFrames);
}

function pushAudioFrame({ channel, pcm, timestampNs }) {
  return binding.push_audio_frame(channel, pcm, timestampNs);
}

function setSourceGain(channel, gain) {
  return binding.set_source_gain(channel, gain);
}

function setSourceMute(channel, mute) {
  return binding.set_source_mute(channel, mute);
}

function monotonicTimeNs() {
  return binding.monotonic_time_ns();
}

module.exports = {
  registerSource,
  pushAudioFrame,
  setSourceGain,
  setSourceMute,
  monotonicTimeNs,
};
