# Local decoder fixtures

These files contain the original mono tone synthesized by `scripts/generate-fixture.ts`. ECHO licenses the source and these encodings under GPL-3.0-or-later. No recordings or user files are included.

The tests generate the WAV locally. The five compressed encodings are checked in so hosted tests need no encoder installation or network fetch. They were generated from `.echo-fixtures/transport.wav` using FFmpeg 9.0.1 (Windows full build) and the following arguments.

```powershell
bun run fixtures:audio
$formats = [ordered]@{ mp3 = 'libmp3lame'; flac = 'flac'; ogg = 'libvorbis'; m4a = 'aac'; aac = 'aac' }
foreach ($entry in $formats.GetEnumerator()) {
    ffmpeg -hide_banner -nostdin -loglevel error -i '.echo-fixtures/transport.wav' -fflags +bitexact -flags:a +bitexact -c:a $entry.Value "tests/fixtures/audio/tone.$($entry.Key)"
}
```

| File        | SHA-256                                                            |
| ----------- | ------------------------------------------------------------------ |
| `tone.aac`  | `facc28c668b3adb4d9dda5c7d033329b4a98ddcef72f0ab1b7ba3942ca3bc199` |
| `tone.flac` | `346fd6d98ccb7f8e0ea021583b64d27fe0b0155271c46afb7296b9733cc7b320` |
| `tone.m4a`  | `68ed69347e2aaa5a837823c2a3fadb508e87ec717364fbfaf7e6f4749b9ac904` |
| `tone.mp3`  | `e36b296e7d054b3dbb56959e7506254f17eee9704b742cf1882fba4f20b68ad7` |
| `tone.ogg`  | `c920e3fd2768ff6fbca5b0aadbc30ac9c7b044a7faa3a756e45bc6943129d540` |
