# Release engineering

ECHO releases are built by GitHub Actions from a tag, attached to a **draft**
GitHub release, and published by a maintainer after review. No script or
workflow publishes on its own.

## Versioning

Semantic versioning. The version appears in three places that must agree:
`package.json`, `src-tauri/Cargo.toml` (and its entry in `Cargo.lock`) and
`src-tauri/tauri.conf.json`. Tags are `v<version>` and point at the exact
commit that was verified. `CHANGELOG.md` gets a section per release.

## Assets

`native/windows-x64/package-release.ps1` produces, from a clean checkout and
a staged decoder rebuilt from source:

| File | Contents |
| --- | --- |
| `ECHO-<v>-windows-x64-setup.exe` | Per-user NSIS installer (no administrator rights) under the production identity `org.echo.player` |
| `ECHO-<v>-source.tar.gz` | `git archive` of the tagged commit |
| `ECHO-<v>-rust-crates.tar.gz` | Locked Cargo dependency sources, including crates needed for offline resolution |
| `ECHO-<v>-js-packages.tar.gz` | npm packages bundled into the renderer |
| `ECHO-<v>-native-source.tar` | Decoder sources, recipes, receipts and notices |
| `ECHO-<v>-license-election.tar.gz` | Decoder license choices, notice patches and the texts they cite |
| `ECHO-<v>-THIRD_PARTY_NOTICES.txt`, `ECHO-<v>-third-party-inventory.json` | Application and renderer notices and component list |
| `ECHO-<v>-release-manifest.json` | Commit, target, decoder digests and tool versions |
| `SHA256SUMS.txt` | SHA-256 of every file above |

The workflow also records GitHub build-provenance attestations for every
asset. Verify a download with:

```powershell
gh attestation verify ECHO-1.0.0-windows-x64-setup.exe --repo Venuss1337/echo
(Get-FileHash ECHO-1.0.0-windows-x64-setup.exe).Hash   # compare with SHA256SUMS.txt
```

## Procedure

1. On a release branch, update the version in the three files, the
   changelog and any documentation that mentions the version. Merge after
   the Verify workflow passes.
2. Review the [decoder license choices](LICENSING.md#decoder-license-choices)
   for any changed native component (regenerate its election patch with
   `scripts/elect-gplv3.ts`), and run the
   [manual acceptance checklist](TESTING.md#manual-acceptance) on the
   candidate where hardware is available. Record what was and was not
   checked.
3. Tag the verified commit (`git tag -s v<version>` if you sign tags) and
   push the tag.
4. Run **Actions → Release** with the tag, and with `redistribution-approved`
   and `draft` enabled once the maintainer has approved the build. The workflow
   runs the full verification suite and security audit, rebuilds the decoder
   from pinned sources and compares it with the committed evidence, runs the
   native suites, checks the license election patches against the pristine
   archives, packages the release, installs and uninstalls the
   installer, verifies checksums, attests the assets and attaches them to a
   draft release.
5. Download the draft's assets, verify checksums and attestations, install
   on a clean Windows account and play local audio and a station.
6. Write the release notes (changes, supported platform, known limitations,
   verification performed) and publish the draft.

Pull requests run `verify.yml`. `native-candidate.yml` rebuilds and tests the
decoder on demand without producing a release.
On a public repository, a build without redistribution approval keeps its
assets on the runner. A private repository can retain build artifacts for
review, with draft attachment gated on the same approval input.

For a local pre-release check, pass the exact installer and staged decoder
manifest to `native/windows-x64/smoke-installer.ps1`. Add `-VerifyWebView`
to exercise the production executable on the private Windows desktop. Use a
test account with no existing production ECHO profile or registration.

## Signing and SmartScreen

Installers are **not code-signed**. Windows SmartScreen may show "Windows
protected your PC" for a new, unsigned download; users choose **More info →
Run anyway** after verifying the checksum or attestation. Signing would need
a certificate the project does not currently have. Release notes must not
describe the installer as signed.

## Updates and data

There is no auto-updater. Installing a newer version over an older one keeps
the user's database, playlists, history and settings; migrations upgrade the
database on first start and never downgrade it, so an older version refuses
a newer database instead of damaging it. The uninstaller keeps user data
unless the user ticks the option to delete it, and never touches music
folders.

## Website

The website is a separate repository. Download links and version text are
updated only after a release has been published.
