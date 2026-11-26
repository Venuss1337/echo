# Native decoder build, installer and release

Read `docs/NATIVE.md`, `docs/RELEASE.md` and `docs/LICENSING.md` first.

- The decoder is built from pinned, hash-checked sources by `build.sh` in a
  pinned MSYS2 UCRT64 toolchain. The `local` profile is the only one that
  ships. Its DLL, link and source inventories in `evidence/local` must match
  a rebuild byte for byte.
- A change to the recipe, a package pin or an FFmpeg/mpv option produces new
  inventories. Commit them with the reason, and expect a new license and
  source review before the result may be distributed.
- Scripts never publish. `package-release.ps1` builds assets; publishing is
  a maintainer decision made on a draft GitHub release.
- Installed paths and test profiles may contain spaces and non-ASCII
  characters on purpose; keep scripts working with them.
- Real-window and installer tests run on a private desktop or a disposable
  identity and must not touch an existing ECHO installation or profile.
