# Zellij Workbench

English | [简体中文](README.zh-CN.md)

Zellij Workbench is a terminal workspace memory manager for local and remote
zellij sessions. It is a ground-up [zellij](https://zellij.dev) port of
[tmux-workbench](https://github.com/LeON-Nie-code/tmux-workbench), aiming for
full feature parity.

It indexes zellij sessions across your machine and SSH servers, remembers the
project context around them, and gives you one fast CLI/TUI entry point to get
back to work.

```bash
zw
```

<p align="center">
  <img src="docs/assets/demo.gif" alt="Zellij Workbench TUI demo" width="100%">
</p>

## Why

SSH plus zellij is resilient, but it does not remember enough when your work
is spread across many machines and many projects. Zellij Workbench adds a
local memory layer above zellij:

- server and connection information
- zellij session and pane snapshot
- project path and active command
- git branch, commit, dirty state, ahead/behind counts, and remote URL
- notes, aliases, tags, archive status, and attach history

It does not replace zellij. It makes zellij workspaces easier to find,
inspect, and resume.

## Features

- Index local zellij sessions and remote zellij sessions over SSH.
- Attach back to a workspace by stable ID: `<server>/<zellij-session>`.
- Manage servers from the CLI.
- Browse workspaces in a TUI with search, server filtering, and view modes.
- Preserve notes, aliases, tags, status, and attach history across scans.
- Detect missing zellij sessions without overwriting archive state, and
  surface zellij's own resurrectable ("exited but not deleted") sessions.
- Capture git repository state for each workspace.
- Refresh in the background without blocking the TUI.
- Store state locally in SQLite.
- Remote scans cost a single SSH round trip regardless of how many sessions
  live on that host.

## Installation

Requirements:

- zellij (0.44.x recommended; both ends of any SSH hop should run matching
  versions, see [Doctor](#doctor--version-skew))
- git
- ssh for remote servers

### Install Script

```bash
curl -fsSL https://raw.githubusercontent.com/LeON-Nie-code/zellij-workbench/main/install.sh | bash
```

The script installs `zw` into `~/.local/bin` by default. Set
`ZELLIJ_WORKBENCH_INSTALL_DIR` to override the install directory.

### Homebrew

```bash
brew tap LeON-Nie-code/zellij-workbench https://github.com/LeON-Nie-code/zellij-workbench
brew install LeON-Nie-code/zellij-workbench/zw
```

### Cargo

```bash
cargo install --git https://github.com/LeON-Nie-code/zellij-workbench zellij-workbench
```

From a local checkout:

```bash
cargo install --path .
```

### Manual Download

Download a binary from the Releases page and place it somewhere in your
`PATH`.

## Quick Start

```bash
zw init
zw servers
zw add-server prod --ssh "ssh prod"
zw scan
zw
```

Attach directly:

```bash
zw attach prod/api
```

## CLI

```bash
zw servers
zw add-server prod --ssh "ssh prod"
zw add-server local-dev --local
zw remove-server prod

zw scan
zw list
zw list --server prod
zw list --status active
zw list --all
zw list --json

zw attach prod/api
zw recreate prod/api

zw note prod/api "Backend uses uv. Frontend is in ./web."
zw alias prod/api api
zw tags prod/api work backend
zw status prod/api archived

zw doctor
zw open-config
```

Remote server commands use your system `ssh`, so existing `~/.ssh/config`,
keys, ProxyCommand, and generated cloud SSH hosts continue to work.

## TUI

```bash
zw
```

Shortcuts:

```text
Enter  attach
/      search
n      edit note in $EDITOR
a      archive or unarchive
v      cycle all / active / archived
s      cycle server filter
z      jump between archived and all
r      rescan
j/k    move
q      quit
```

Search supports plain text and filters:

```text
server:prod status:active tag:backend git:dirty
```

Git filters include `dirty`, `clean`, `remote`, `ahead`, `behind`, branch text,
commit text, and remote URL text.

## Configuration

Config file:

```text
~/.config/zw/config.yaml
```

Example:

```yaml
servers:
  - name: local
    ssh: ""
    term: xterm-256color
    local: true
  - name: prod
    ssh: ssh prod
    term: xterm-256color
    local: false
```

Local index:

```text
~/.local/share/zw/workspaces.db
```

Both paths can be overridden with `ZW_CONFIG_DIR` and `ZW_DATA_DIR`, which is
mainly useful for tests and for running more than one isolated instance.

## Doctor & version skew

Unlike tmux, zellij expects the client and server to be running compatible
versions; a mismatched remote binary can make `attach` fail in confusing
ways. `zw doctor` compares your local `zellij --version` against each
configured server's and warns on a mismatch:

```text
server: prod
  ssh: ok
  host: prod.example.com
  zellij: ok
  zellij version: 0.43.1
  warning: local zellij 0.44.3 differs from 0.43.1 on prod (attach may fail)
```

## Architecture

Zellij Workbench reads zellij state, stores a local index, and uses
zellij/ssh for attach and discovery.

```text
zellij list-sessions + action list-panes (1 SSH call)  ->  zw scan  ->  SQLite index  ->  CLI/TUI
                                              git status (1 SSH call per workspace)  ->
```

Stack:

- Rust
- clap for CLI parsing
- ratatui + crossterm for TUI
- rusqlite for the local index
- system `ssh`, `zellij`, and `git`

`zw` never links against zellij's internal Rust crates or its WASM plugin
API — it only shells out to the `zellij` CLI, exactly like tmux-workbench
shells out to `tmux`. That keeps it decoupled from zellij's still-evolving
internal API and lets it manage sessions across machines it is not attached
to, which a session-local WASM plugin could not do.

## Differences from tmux-workbench

This is a deliberate, tracked list rather than an accident of translation:

- **Session discovery**: tmux exposes `list-panes -a -F ...` as one call for
  every session on the server. zellij has no server-wide equivalent, so `zw`
  enumerates sessions and queries each one's panes in a single batched SSH
  script instead of one round trip per session.
- **`pane_command` is a full command line** (e.g. `claude --resume`), not a
  bare process name like tmux's `pane_current_command`. `zw` normalizes it to
  the first token before agent detection and display.
- **`recreate` is idempotent**: it runs `zellij attach <session> --create`
  instead of hand-rolling a `new-session -A` shell command.
- **Presence has a third state**: a `resurrectable` flag tracks zellij
  sessions that exited but can still be resurrected by attaching, shown as a
  `*` suffix in the TUI/CLI status column.
- **`zw doctor` checks version skew** between the local zellij client and
  each remote server's zellij binary, which tmux does not require.

## Status

Zellij Workbench is pre-1.0. The CLI and database format may still change.

Implemented:

- local and remote zellij session indexing
- concurrent scan with command timeouts, batched into one SSH round trip per
  host for session/pane discovery
- TUI auto-refresh with visible scan status
- server management CLI
- workspace notes, aliases, tags, archive status
- presence tracking for missing and resurrectable zellij sessions
- attach history
- git snapshots
- structured list and JSON output
- version-skew warnings in `zw doctor`

Planned:

- structured layout snapshot/restore using `zellij action dump-layout`
- `zellij web` as an SSH-free remote transport option
- richer query filters and saved views

See [ROADMAP.md](ROADMAP.md) for more detail.

## Testing

```bash
cargo test
```

The suite includes real end-to-end coverage, not just unit tests:

- `tests/local_scan.rs` drives the actual `zellij` binary: it creates a real
  background session against a real git repo, scans it through the compiled
  `zw` binary, and checks presence/git/user-metadata behavior across
  rescans and after the session disappears.
- `tests/multi_host_ssh.rs` spins up two throwaway, unprivileged loopback
  `sshd` instances (no `sudo`, no system "Remote Login") to simulate two
  remote machines, then verifies `zw` aggregates and namespaces sessions from
  both correctly and costs the expected number of SSH round trips per scan.
  See the module doc comment in that file for the one caveat it can't fully
  simulate on a single dev machine (independent zellij session storage per
  host — zellij keys its socket directory off the OS per-UID temp
  directory, not `$HOME`).

Both are skipped gracefully (not failed) on a machine without `zellij` or
`sshd`/`ssh-keygen` available.

## Further reading

[docs/tui-development-guide.md](docs/tui-development-guide.md) — an in-depth
breakdown of TUI application architecture (event loops, rendering
performance, layout, navigation state, async responsiveness, and the
features a good TUI needs), using this repo and lazygit as worked examples.

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md),
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), and [SECURITY.md](SECURITY.md).

## License

MIT. See [LICENSE](LICENSE).
