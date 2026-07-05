# Demo

This page tracks the demo assets embedded in the README.

## Current Assets

- `docs/assets/demo.gif`
- `docs/demo/workbench.tape`
- `scripts/demo-fixture.sh`

The GIF is generated with [VHS](https://github.com/charmbracelet/vhs) from the
real `zw` binary and a local fixture database (`scripts/demo-fixture.sh`
seeds a throwaway `config.yaml` + `workspaces.db` with realistic but fake
workspaces). It does not use mocked screenshots.

The tape unsets `NO_COLOR` before launching `zw`. Keep that line in place:
without it, crossterm correctly disables ANSI colors and the GIF becomes a
flat grayscale recording.

Regenerate it from the repository root:

```bash
cargo build --release
PATH="$PWD/target/release:$PATH" vhs docs/demo/workbench.tape
```

If VHS fails with `could not open ttyd: navigation failed:
net::ERR_CONNECTION_REFUSED`, it's a startup race between `ttyd` and the
headless browser VHS drives — just retry.

Suggested flow (already scripted in the tape):

1. `zw` — launch the TUI against the fixture data
2. search with `server:prod git:dirty`
3. cycle server (`s`) and view (`v`) filters, move with `j`/`k`
4. `q` to quit
