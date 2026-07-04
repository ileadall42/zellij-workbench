# Security Policy

Zellij Workbench indexes local metadata (session names, working directories,
git state) into a local SQLite database at `~/.local/share/zw/workspaces.db`
and shells out to `ssh`/`zellij`/`git` using your existing credentials. It
does not transmit any data off your machine or the hosts you configure.

## Reporting a Vulnerability

If you find a security issue (e.g. a shell-injection path through a
workspace name, note, or server config value), please open a private report
via GitHub's "Report a vulnerability" flow on this repository rather than a
public issue, so a fix can go out before details are public.

## Scope notes

- Server SSH commands in `config.yaml` are executed as-is; treat that file
  with the same trust as your shell profile.
- All shell arguments built from workspace/session data are passed through
  `shell_quote` (single-quoted, with embedded quotes escaped) before being
  interpolated into a command string. New code that builds shell commands
  should keep doing the same.
