# Contributing to ECHO

Thank you for helping. Bug reports, fixes, tests, accessibility work and
documentation are all welcome. This page explains how to get a change
merged.

## Before you start

- **Bugs:** search the issues first, then open one with the bug template:
  ECHO version, Windows version, steps, what you expected and what happened.
  Attach the tray's *Export diagnostics* snapshot if relevant. Do not attach
  music files or personal paths.
- **Features:** open an issue describing the problem before writing code.
  ECHO's interface (Atmosphere 2.0, see [docs/DESIGN.md](docs/DESIGN.md)) is
  deliberate; proposals that restyle it are unlikely to be accepted, while
  changes that fix defects or extend it in its own terms are welcome.
- **Security issues:** never in public issues; see [SECURITY.md](SECURITY.md).

## Making a change

1. Set up the toolchain with [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).
2. Read [AGENTS.md](AGENTS.md); it is the short list of rules every change
   follows, for people and coding agents alike, and it points to the one or
   two documents relevant to your area.
3. Branch from `main`, keep the change to one coherent purpose, and add tests
   at the lowest level that can observe the behaviour, including its failure
   cases.
4. Run `bun run verify` (and `bun run test:security` if you changed
   dependencies). If you touched the decoder, installer or window code, run
   the relevant suite from [docs/TESTING.md](docs/TESTING.md).
5. Open a pull request using the template. List the commands you ran and
   their results; say clearly what you could not run.

## What reviewers look for

- Rust owns behaviour; the renderer presents it. No new renderer power
  without a reason recorded in the pull request.
- No user data or music file can be lost by the change.
- Tests are deterministic and meaningful. Tests, lint rules, security checks
  and visual baselines are never weakened to get a green build.
- Comments explain why (invariants, ownership, `unsafe`, security), not what.
- Documentation describing the behaviour is updated in the same change.
- New dependencies are justified and pass the license policy in `deny.toml`.

Reviewers may ask for changes for architectural drift, missing tests or
unnecessary complexity even when a pull request works.

## AI-assisted contributions

AI coding assistants are welcome; `AGENTS.md` and `CLAUDE.md` are written so
tools such as Codex and Claude Code can work in this repository directly.
The contributor who opens the pull request is responsible for every line:
you must understand it, have reviewed it, have run the tests you report, and
be able to maintain it. AI-assisted work is held to exactly the same
standard as any other. Do not add tool co-author trailers, generated-by
signatures or promotional text to commits; write normal commit messages.

## Commits

Short imperative subject lines ("Fix seek after radio replacement"), with a
body explaining why when it is not obvious. Keep unrelated formatting and
refactoring out of functional changes.

## License

By contributing you agree that your contribution is licensed under the GNU
General Public License, version 3 or later, like the rest of ECHO. Only
submit code, assets and fixtures you have the right to license this way.
