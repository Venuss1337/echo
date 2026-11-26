@AGENTS.md

## Claude Code notes

- Subdirectories carry their own `AGENTS.md`; the `CLAUDE.md` beside each one
  imports it, so the rules load when you work in that directory.
- Use the Bash tool with PowerShell semantics for repository scripts:
  `pwsh -NoProfile -File native/windows-x64/<script>.ps1 ...`.
- Long native builds and real-window suites take minutes; run them in the
  background and keep working on independent steps.
