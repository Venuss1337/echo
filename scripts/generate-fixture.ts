// Original synthesized audio. Fixtures are covered by the repository GPL-3.0-or-later license.
async function writeTone(name: string, seconds: number) {
  const rate = 8_000;
  const samples = rate * seconds;
  const dataSize = samples * 2;
  const bytes = Buffer.alloc(44 + dataSize);
  bytes.write("RIFF", 0);
  bytes.writeUInt32LE(bytes.length - 8, 4);
  bytes.write("WAVEfmt ", 8);
  bytes.writeUInt32LE(16, 16);
  bytes.writeUInt16LE(1, 20);
  bytes.writeUInt16LE(1, 22);
  bytes.writeUInt32LE(rate, 24);
  bytes.writeUInt32LE(rate * 2, 28);
  bytes.writeUInt16LE(2, 32);
  bytes.writeUInt16LE(16, 34);
  bytes.write("data", 36);
  bytes.writeUInt32LE(dataSize, 40);
  for (let sample = 0; sample < samples; sample++) {
    const fade = Math.min(1, sample / 80, (samples - sample - 1) / 80);
    const amplitude = Math.sin((2 * Math.PI * 440 * sample) / rate) * 2_000;
    bytes.writeInt16LE(Math.round(amplitude * fade), 44 + sample * 2);
  }
  await Bun.write(`.echo-fixtures/${name}`, bytes);
}
await writeTone("tone.wav", 1);
await writeTone("transport.wav", 12);
