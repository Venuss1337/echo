# Native decoder

ECHO's audio engine is libmpv 0.42.0 with FFmpeg 9.1 and libplacebo 7.387.0,
built from pinned source archives in a pinned MSYS2 UCRT64 toolchain on
Windows. ECHO builds it itself rather than downloading a prebuilt DLL, so
that every input is known, the security profile is exactly the one ECHO
needs, and the corresponding source can be published with each release.

Everything lives in `native/windows-x64/`.

## Inputs

| Input | Pin |
| --- | --- |
| MSYS2 base | `msys2-base-x86_64-20261120.tar.xz`, SHA-256 in `prepare.ps1` |
| Toolchain and libraries | GCC 16.2.0, Meson, Ninja, NASM and the linked MinGW packages, each with its SHA-256 in `packages.sha256`; package signatures are verified on install |
| FFmpeg | `ffmpeg-9.1.tar.xz` from ffmpeg.org, SHA-256 `1a4d394a…2433` |
| mpv | commit `b0841e8c4d156b8403dc1876c4cbd9011eb583f3` (tag v0.42.0), SHA-256 `a4a79fa6…02e4` |
| libplacebo | tag v7.387.0 (`275f7dea…`), SHA-256 `3bf5d727…0d74` |

`build.sh` checks the three source archives again before extracting them,
builds in fresh directories with a fixed install prefix, `SOURCE_DATE_EPOCH`,
`LC_ALL=C` and `TZ=UTC`, and inspects the resulting PE files for ASLR,
high-entropy VA and NX.

## The `local` profile

The shipped profile is `local`:

- FFmpeg is configured with `--disable-network --disable-everything
  --disable-protocols`, static libraries, GPL version 3, and only the AAC,
  FLAC, MOV, MP3, Ogg and WAV demuxers with their audio decoders.
- mpv is a shared libmpv with WASAPI, Windows threads, scripting and the
  command-line player compiled out. Its stream registry is patched to the
  read-only callback handler only, and its demuxer registry to lavf.
- libplacebo (required by mpv at configure time) is built static with
  optional features, Dolby Vision support and libdovi disabled.

The result is three DLLs: `libmpv-2.dll` and the two MinGW runtime DLLs it
needs. Their SHA-256 digests are in `evidence/local/dlls.json`; the PE import
closure, every linked archive and startup object, and their owning MSYS2
packages are in `evidence/local/inventory.json`; and the matching MSYS2
source archives in `evidence/local/source-inventory.json`.

The `feasibility` profile (network-capable FFmpeg, extra demuxers) exists
only to keep earlier transport tests reproducible. It is never packaged.

## Reproducibility

Independent build roots produce byte-identical DLLs. CI rebuilds the decoder
from scratch (`.github/workflows/native-candidate.yml` and the release
workflow) and fails if the staged `dlls.json`, `inventory.json` or
`source-inventory.json` differs from the committed evidence. A deliberate
change to the recipe, a pin or an option therefore shows up as a diff to
those files, which must be committed with the reason.

## Build, stage and test

Use PowerShell 7 and a short ASCII work path (some MSYS2 tools cannot handle
others); the staged and installed paths may contain spaces and Unicode.

```powershell
$root  = 'C:/echo-native'                 # new, empty
$stage = 'C:/echo-native-stage'           # new, empty
./native/windows-x64/prepare.ps1 -Root $root
$env:MSYSTEM = 'UCRT64'
& "$root/msys64/usr/bin/bash.exe" -l -c 'root="$(cygpath -u "$1")"; bash "$root/build.sh" "$root" 4 local' echo $root
./native/windows-x64/stage.ps1 -Root $root -Destination $stage -LocalOnly
./native/windows-x64/collect-sources.ps1 -ResourceRoot $stage -SourceCache "$root/package-sources"
./native/windows-x64/test-local.ps1 -ResourceRoot $stage
./native/windows-x64/test-live.ps1 -ResourceRoot $stage
./native/windows-x64/collect-notices.ps1 -Root $root -ResourceRoot $stage -SourceCache "$root/package-sources"
```

A full build takes roughly 15–25 minutes on a hosted runner; allow at least 8 GB of free disk. With a staged
decoder, run the desktop app with audio:

```powershell
$env:ECHO_NATIVE_DLL_MANIFEST = "$stage/dlls.json"
bun run tauri build --debug --features native-resources -- --locked
```

Packaging and release are described in [Release](RELEASE.md).

## Loading at runtime

With the `native-resources` feature, the build embeds the digest manifest in
the executable. At startup, off the UI thread, the host checks the exact file
set in the installation's `native` directory, each SHA-256, the client ABI
and the required exports. Windows resolves the DLLs' imports only from that
directory and System32; PATH, the working directory and writable caches are
never searched. The files stay open with read-only sharing so they cannot be
replaced while mapped, and a module already loaded under a colliding name is
refused. A build without the feature, or with any mismatch, runs with audio
disabled.

## Test-only probe

`scripts/prepare-native-probe.ps1` downloads the official mpv v0.42.0
`libmpv` release asset (signed archive, pinned SHA-256) into the user
profile so `bun run test:native` can exercise the FFI on any machine and in
the Verify workflow. That DLL is never packaged or distributed by ECHO.

## Source and notices

`collect-notices.ps1` gathers the license texts of every linked package and
of mpv, FFmpeg and libplacebo. `package-sources.ps1` assembles the native
corresponding-source archive: the three upstream archives, the MSYS2 source
archives of every linked package (checked against the inventory), the
recipes in this directory, FFmpeg's generated configuration, Meson options,
the link map, build receipts, notices and per-file checksums. See
[Licensing](LICENSING.md).
