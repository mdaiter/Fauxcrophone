const {
  registerSource,
  pushAudioFrame,
  setSourceGain,
  monotonicTimeNs,
} = require('../index');

const SAMPLE_RATE = 48_000;
const BLOCK = 256;
let phase = 0;

if (!registerSource(1, 8192)) {
  console.error('Loopback mixer not initialised. Launch the host app/driver first.');
  process.exit(1);
}

setSourceGain(1, 1.0);

function renderBlock() {
  const buffer = new Float32Array(BLOCK * 2);
  const step = (440 / SAMPLE_RATE) * Math.PI * 2;
  for (let i = 0; i < BLOCK; i++) {
    const sample = Math.sin(phase) * 0.2;
    buffer[i * 2] = sample;
    buffer[i * 2 + 1] = sample;
    phase += step;
  }
  const timestamp = Number(monotonicTimeNs());
  if (!pushAudioFrame({ channel: 1, pcm: buffer, timestampNs: timestamp })) {
    console.warn('Loopback mixer queue full; frame dropped');
  }
}

setInterval(renderBlock, (BLOCK / SAMPLE_RATE) * 1000);

console.log('Feeding synthetic audio into loopback mixer...');
