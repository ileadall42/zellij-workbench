# Roadmap

Zellij Workbench is currently focused on reaching full parity with
tmux-workbench and becoming a dependable daily driver for SSH plus zellij
development.

## Near Term

- Validate the `resurrectable` presence state machine against more real
  exit paths (crash, network drop, explicit kill) and simplify it back to a
  two-state model if the transitions prove unreliable in practice.
- Improve TUI layout on small terminals.
- Add release binaries for Linux and macOS.

## Later

- Structured layout snapshot/restore using `zellij action dump-layout` and
  `zellij --new-session-with-layout`, going beyond tmux-workbench's own
  (still unimplemented) pane-layout-restore goal.
- Offer `zellij web` as an SSH-free remote transport.
- Support project-local notes such as `.zellij-workbench.md`.
- Add richer query filters and saved views.
- Consider a Homebrew tap once releases are stable.

## Non Goals

- Replacing zellij.
- Managing cloud infrastructure.
- Syncing private workspace state through a hosted service.
