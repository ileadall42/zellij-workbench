# Contributing

Thanks for considering a contribution to Zellij Workbench.

## Development

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

`cargo test` includes end-to-end tests that drive a real local `zellij`
binary and two simulated remote hosts over loopback SSH (see the Testing
section of the README). They skip themselves on machines missing `zellij`
or `sshd`/`ssh-keygen`, but on a machine that has them, running the suite
will transiently create and destroy real zellij sessions named with a
`zw-it-` prefix.

## Guidelines

- Keep `zellij.rs` as the only place that shells out to the `zellij` binary;
  everything else should stay zellij-agnostic so it stays easy to reason
  about and to test with plain data.
- Prefer small, focused commits with descriptive messages.
- Add or update tests for behavior changes — this project treats the
  integration tests as first-class, not an afterthought.
- Run `cargo clippy --all-targets -- -D warnings` before opening a PR.

## Reporting bugs

Please include your zellij version (`zellij --version`), your OS, and the
output of `zw doctor` where relevant.
