# Contributing

Thanks for considering a contribution to Zellij Workbench. This document is
the actual bar PRs are held to, not a formality — following it means your
PR is reviewable on the first pass instead of going back and forth.

## Before you start

- For a bug fix or something small, just open a PR.
- For anything that changes behavior, adds a command/flag, or touches the
  scan/attach/recreate flow, open an issue first (or comment on an existing
  one) so the approach can be agreed on before you invest time in it.
- Check open issues and PRs first so effort doesn't collide. Issues labeled
  `good first issue` are scoped for a first contribution; `help wanted`
  means it's not currently anyone's priority but is welcome.
- Questions and design discussion belong in
  [Discussions](https://github.com/ileadall42/zellij-workbench/discussions);
  keep Issues for actionable bugs/features.

## Development setup

Requirements: Rust (stable), `zellij`, `git`, and — only for the SSH
simulation test — `sshd`/`ssh-keygen` (already present on macOS/Linux; no
`sudo` or system "Remote Login" needed, see `tests/support/mod.rs`).

```bash
cargo build
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

All four are required and are exactly what CI runs — if they pass locally,
CI passes. `cargo test` includes end-to-end tests that drive a real local
`zellij` binary and two simulated remote hosts over loopback SSH (see the
Testing section of the README). They skip themselves (print a message,
don't fail) on machines missing `zellij` or `sshd`/`ssh-keygen`, but where
those are present, running the suite will transiently create and destroy
real zellij sessions named with a `zw-it-` prefix.

## Code standards

- **Formatting is `cargo fmt`, not personal preference.** Run it before
  committing; CI rejects unformatted code.
- **Clippy warnings are errors** (`-D warnings`). If a lint is genuinely
  wrong for a specific line, `#[allow(...)]` it right there with a comment
  explaining why, rather than disabling it project-wide.
- **`src/zellij.rs` is the only module that shells out to the `zellij`
  binary.** Everything else (`db.rs`, `commands.rs`, `tui.rs`, `model.rs`)
  stays zellij-agnostic, working with plain data, so it stays easy to unit
  test without spawning real sessions. If you're adding a new zellij CLI
  interaction, it belongs there.
- **No unexplained comments.** A comment should carry a reason a reader
  couldn't get from the code itself (a non-obvious constraint, a workaround,
  why an alternative was rejected) — not a restatement of what the next
  line does.
- **Don't add error handling, abstractions, or config options for cases
  that can't happen yet.** Match the existing code's directness: it's a
  small, focused tool, not a framework.

## Tests

This project treats the integration tests as first-class, not an
afterthought:

- Behavior changes need a test. A bug fix should come with a test that
  fails without the fix.
- Prefer extending `tests/local_scan.rs` (real local zellij) or
  `tests/multi_host_ssh.rs` (simulated remote hosts) over hand-waving
  "manually verified" in a PR description — anyone reviewing or later
  refactoring needs to be able to re-run the proof, not just read a claim.
- If you add a shared test helper, put it in `tests/support/mod.rs` rather
  than duplicating setup code across test files.

## Commit messages and PRs

- [Conventional Commits](https://www.conventionalcommits.org/):
  `feat: ...`, `fix: ...`, `docs: ...`, `test: ...`, `refactor: ...`,
  `chore: ...`. Look at `git log` for the tone/format actually in use here.
- Keep commits focused — one logical change per commit — but don't feel
  obligated to split a PR into many commits just for the sake of it.
- PR description: what changed and why, plus how you verified it (which
  commands you ran, or which new test covers it). Link the issue if there
  is one.
- A PR that changes behavior without a corresponding test, or that fails
  `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`, will
  get review comments asking for those before anything else — save the
  round-trip by checking locally first.

## Reporting bugs

Please include your zellij version (`zellij --version`, and the remote's
version too if it's SSH-related), your OS, and the output of `zw doctor`
where relevant.
