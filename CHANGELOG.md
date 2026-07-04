# Changelog

All notable changes to Zellij Workbench will be documented in this file.

The project is currently pre-1.0. Breaking changes may happen while the CLI
and configuration format settle.

## Unreleased

- Port tmux-workbench's indexing/CLI/TUI architecture to zellij.
- Add the `zw` CLI and TUI.
- Batch remote session/pane discovery into a single SSH round trip per host
  instead of one per session.
- Add a `resurrectable` presence flag for zellij sessions that exited but can
  still be resurrected by attaching.
- Add version-skew detection between the local zellij client and each
  configured server in `zw doctor`.
- Add server management commands.
- Add notes, aliases, tags, archive status, and attach history.
- Add git snapshots for branch, commit, dirty state, ahead/behind, and remote.
- Add concurrent scan with command timeouts.
- Add TUI scan status, server filtering, structured search, and view modes.
- Add end-to-end tests driving a real local zellij session and two simulated
  remote hosts over loopback SSH.
