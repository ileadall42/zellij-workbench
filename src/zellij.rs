use std::{
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Deserialize;

use crate::{
    model::{DoctorReport, GitInfo, Pane, ServerConfig, Workspace},
    util::shell_quote,
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(8);
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(50);

const SESSION_BEGIN: &str = "##ZW-SESSION-BEGIN##";
const SESSION_END: &str = "##ZW-SESSION-END##";
const SCAN_ERROR: &str = "##ZW-SCAN-ERROR##";

/// One `zellij action list-panes --all --json` entry. Field presence differs
/// between plugin panes (tab-bar, status-bar, ...) and terminal panes, so the
/// terminal-only fields are optional.
#[derive(Debug, Deserialize)]
struct PaneJson {
    id: i64,
    is_plugin: bool,
    #[serde(default)]
    is_focused: bool,
    #[serde(default)]
    is_floating: bool,
    #[serde(default)]
    title: String,
    #[serde(default)]
    tab_position: i64,
    #[serde(default)]
    tab_name: String,
    #[serde(default)]
    pane_command: Option<String>,
    #[serde(default)]
    pane_cwd: Option<String>,
}

pub fn scan_server(server: &ServerConfig) -> Result<Vec<Workspace>> {
    let output = run_server_command(server, &scan_script()).context("failed to run zellij scan")?;
    let raw = String::from_utf8_lossy(&output.stdout);
    // The script reports its own errors (e.g. zellij missing) on stdout via
    // the ##ZW-SCAN-ERROR## marker; only fall back to raw stderr (mostly SSH
    // connection banners) when the script never got to run at all.
    if !output.status.success() && !raw.contains(SCAN_ERROR) {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let sessions = parse_scan_output(&raw)?;

    let mut workspaces: Vec<Workspace> = sessions
        .into_iter()
        .map(|(name, resurrectable, panes)| {
            build_workspace(&server.name, &name, resurrectable, panes)
        })
        .collect();

    for workspace in &mut workspaces {
        if workspace.root_path.is_empty() {
            continue;
        }
        workspace.git = scan_git(server, &workspace.root_path).ok().flatten();
    }
    Ok(workspaces)
}

/// Single shell script covering session enumeration and per-session pane
/// listing, so a remote scan costs one SSH round trip instead of one per
/// session.
fn scan_script() -> String {
    "if ! command -v zellij >/dev/null 2>&1; then \
       printf '##ZW-SCAN-ERROR## zellij not found in PATH\\n'; exit 1; \
     fi; \
     sessions_output=$(zellij list-sessions --no-formatting 2>&1); \
     sessions_status=$?; \
     if [ \"$sessions_status\" -ne 0 ]; then \
       case \"$sessions_output\" in \
         *\"No active zellij sessions\"*) sessions_output='' ;; \
         *) printf '##ZW-SCAN-ERROR## %s\\n' \"$sessions_output\"; exit 1 ;; \
       esac; \
     fi; \
     printf '%s\\n' \"$sessions_output\" | while IFS= read -r line; do \
       [ -z \"$line\" ] && continue; \
       name=$(printf '%s\\n' \"$line\" | awk '{print $1}'); \
       [ -z \"$name\" ] && continue; \
       case \"$line\" in \
         *EXITED*) resurrectable=1 ;; \
         *) resurrectable=0 ;; \
       esac; \
       printf '##ZW-SESSION-BEGIN## %s %s\\n' \"$name\" \"$resurrectable\"; \
       if [ \"$resurrectable\" = \"0\" ]; then \
         zellij --session \"$name\" action list-panes --all --json 2>/dev/null; \
       fi; \
       printf '##ZW-SESSION-END##\\n'; \
     done"
        .to_string()
}

fn parse_scan_output(raw: &str) -> Result<Vec<(String, bool, Vec<PaneJson>)>> {
    if let Some(error_line) = raw.lines().find(|line| line.starts_with(SCAN_ERROR)) {
        let message = error_line.trim_start_matches(SCAN_ERROR).trim().to_string();
        bail!("{message}");
    }

    let mut sessions = Vec::new();
    let mut current: Option<(String, bool, String)> = None;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix(SESSION_BEGIN) {
            let rest = rest.trim();
            let mut parts = rest.splitn(2, ' ');
            let name = parts.next().unwrap_or_default().to_string();
            let resurrectable = parts.next().unwrap_or("0").trim() == "1";
            current = Some((name, resurrectable, String::new()));
        } else if line.trim() == SESSION_END {
            if let Some((name, resurrectable, buf)) = current.take() {
                let panes: Vec<PaneJson> = if buf.trim().is_empty() {
                    Vec::new()
                } else {
                    serde_json::from_str(&buf).unwrap_or_default()
                };
                sessions.push((name, resurrectable, panes));
            }
        } else if let Some((_, _, buf)) = current.as_mut() {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    Ok(sessions)
}

/// `pane_command` is the full command line (e.g. "claude --resume"), unlike
/// tmux's `pane_current_command` which is just the process name. Reduce it to
/// the first token so agent detection and display match tmux's semantics.
/// zellij also wraps login shells as "(bash)"; strip that decoration too.
fn process_name(pane_command: Option<&str>) -> String {
    let token = pane_command
        .and_then(|command| command.split_whitespace().next())
        .unwrap_or("");
    token
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(token)
        .to_string()
}

fn build_workspace(
    server: &str,
    session: &str,
    resurrectable: bool,
    panes_json: Vec<PaneJson>,
) -> Workspace {
    let panes: Vec<Pane> = panes_json
        .into_iter()
        .filter(|pane| !pane.is_plugin)
        .map(|pane| Pane {
            pane_id: format!("terminal_{}", pane.id),
            tab_name: pane.tab_name,
            tab_position: pane.tab_position,
            pane: pane.id,
            active: pane.is_focused,
            is_floating: pane.is_floating,
            command: process_name(pane.pane_command.as_deref()),
            path: pane.pane_cwd.unwrap_or_default(),
            title: pane.title,
        })
        .collect();

    let active = panes
        .iter()
        .find(|pane| pane.active)
        .or_else(|| panes.first());
    let agent_pane = panes
        .iter()
        .find(|pane| pane.command == "codex" || pane.command == "claude")
        .or(active);
    let agent = agent_pane
        .map(|pane| pane.command.clone())
        .unwrap_or_default();
    let root_path = agent_pane.map(|pane| pane.path.clone()).unwrap_or_default();

    Workspace {
        id: format!("{server}/{session}"),
        name: session.to_string(),
        alias: None,
        server: server.to_string(),
        session: session.to_string(),
        root_path,
        agent,
        panes,
        note: String::new(),
        status: "active".to_string(),
        presence: "seen".to_string(),
        resurrectable,
        tags: Vec::new(),
        last_seen: Utc::now().to_rfc3339(),
        last_attached_at: None,
        attach_count: 0,
        git: None,
    }
}

pub fn remote_session_exists(server: &ServerConfig, session: &str) -> Result<bool> {
    let command = "zellij list-sessions --no-formatting 2>/dev/null || true".to_string();
    let output = run_server_command(server, &command)?;
    let found = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .any(|name| name == session);
    Ok(found)
}

pub fn remote_doctor(server: &ServerConfig) -> Result<DoctorReport> {
    let command = "printf 'hostname='; hostname; \
        if command -v zellij >/dev/null 2>&1; then \
          echo 'zellij=ok'; \
          printf 'zellij_version='; zellij --version 2>/dev/null | awk '{print $2}'; \
          zellij list-sessions --no-formatting 2>/dev/null | while IFS= read -r line; do \
            name=$(printf '%s\\n' \"$line\" | awk '{print $1}'); \
            [ -n \"$name\" ] && printf 'session=%s\\n' \"$name\"; \
          done; \
        else \
          echo 'zellij=missing'; \
        fi";
    let output = run_server_command(server, command)?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }

    let mut hostname = String::from("unknown");
    let mut zellij_available = false;
    let mut zellij_version = None;
    let mut sessions = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(value) = line.strip_prefix("hostname=") {
            hostname = value.to_string();
        } else if let Some(value) = line.strip_prefix("zellij_version=") {
            if !value.is_empty() {
                zellij_version = Some(value.to_string());
            }
        } else if let Some(value) = line.strip_prefix("zellij=") {
            zellij_available = value == "ok";
        } else if let Some(value) = line.strip_prefix("session=") {
            sessions.push(value.to_string());
        }
    }

    Ok(DoctorReport {
        hostname,
        zellij_available,
        zellij_version,
        sessions,
    })
}

pub fn local_zellij_version() -> Option<String> {
    let output = Command::new("zellij").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(1)
        .map(ToString::to_string)
}

fn run_server_command(server: &ServerConfig, command: &str) -> Result<Output> {
    if server.local {
        return run_command_with_timeout(Command::new("sh").arg("-lc").arg(command), "local");
    }

    let command = format!("{} {}", server.ssh, shell_quote(command));
    run_command_with_timeout(
        Command::new("sh").arg("-lc").arg(command),
        &format!("remote {}", server.name),
    )
}

fn run_command_with_timeout(command: &mut Command, label: &str) -> Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to run {label} command"))?;
    let started = Instant::now();

    loop {
        if child
            .try_wait()
            .with_context(|| format!("failed to wait for {label} command"))?
            .is_some()
        {
            return child
                .wait_with_output()
                .with_context(|| format!("failed to collect {label} command output"));
        }

        if started.elapsed() >= COMMAND_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait_with_output();
            bail!(
                "{label} command timed out after {}s",
                COMMAND_TIMEOUT.as_secs()
            );
        }

        thread::sleep(COMMAND_POLL_INTERVAL);
    }
}

fn scan_git(server: &ServerConfig, path: &str) -> Result<Option<GitInfo>> {
    let command = format!(
        "cd {} 2>/dev/null && git rev-parse --is-inside-work-tree >/dev/null 2>&1 && branch=$(git branch --show-current 2>/dev/null || true) && head=$(git rev-parse --short HEAD 2>/dev/null || true) && remote=$(git remote get-url origin 2>/dev/null || true) && if [ -n \"$(git status --porcelain 2>/dev/null)\" ]; then dirty=1; else dirty=0; fi && counts=$(git rev-list --left-right --count '@{{upstream}}...HEAD' 2>/dev/null || printf '0\t0') && printf 'branch=%s\\nhead=%s\\nremote=%s\\ndirty=%s\\ncounts=%s\\n' \"$branch\" \"$head\" \"$remote\" \"$dirty\" \"$counts\"",
        shell_quote(path)
    );
    let output = run_server_command(server, &command)?;
    if !output.status.success() {
        return Ok(None);
    }

    let mut branch = None;
    let mut head = None;
    let mut remote = None;
    let mut dirty = false;
    let mut ahead = 0;
    let mut behind = 0;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(value) = line.strip_prefix("branch=") {
            branch = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
        } else if let Some(value) = line.strip_prefix("head=") {
            head = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
        } else if let Some(value) = line.strip_prefix("remote=") {
            remote = normalize_git_remote(value);
        } else if let Some(value) = line.strip_prefix("dirty=") {
            dirty = value == "1";
        } else if let Some(value) = line.strip_prefix("counts=") {
            let mut parts = value.split_whitespace();
            behind = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
            ahead = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
        }
    }

    Ok(Some(GitInfo {
        branch,
        head,
        remote,
        dirty,
        ahead,
        behind,
    }))
}

fn normalize_git_remote(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if let Some((host, path)) = value
        .strip_prefix("git@")
        .and_then(|rest| rest.split_once(':'))
    {
        return Some(format!(
            "https://{}/{}",
            host,
            path.trim_end_matches(".git")
        ));
    }

    if let Some(rest) = value.strip_prefix("ssh://git@") {
        if let Some((host, path)) = rest.split_once('/') {
            return Some(format!(
                "https://{}/{}",
                host,
                path.trim_end_matches(".git")
            ));
        }
    }

    Some(value.trim_end_matches(".git").to_string())
}

pub fn zellij_attach_command(session: &str, term: Option<&str>) -> String {
    let term = term.unwrap_or("xterm-256color");
    format!(
        "TERM={} zellij attach {}",
        shell_quote(term),
        shell_quote(session)
    )
}

pub fn zellij_create_command(session: &str, term: Option<&str>) -> String {
    let term = term.unwrap_or("xterm-256color");
    format!(
        "TERM={} zellij attach {} --create",
        shell_quote(term),
        shell_quote(session)
    )
}

pub fn attach_ssh_command(ssh: &str) -> String {
    let trimmed = ssh.trim();
    if trimmed == "ssh" {
        return "ssh -t".to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("ssh ") {
        if rest
            .split_whitespace()
            .any(|part| part == "-t" || part == "-tt")
        {
            trimmed.to_string()
        } else {
            format!("ssh -t {rest}")
        }
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::model::ServerConfig;

    use super::{
        attach_ssh_command, build_workspace, normalize_git_remote, parse_scan_output, process_name,
        zellij_attach_command, zellij_create_command,
    };

    fn server(name: &str, local: bool) -> ServerConfig {
        ServerConfig {
            name: name.to_string(),
            ssh: if local {
                String::new()
            } else {
                format!("ssh {name}")
            },
            term: Some("xterm-256color".to_string()),
            local,
        }
    }

    #[test]
    fn adds_tty_to_plain_ssh_attach_command() {
        assert_eq!(attach_ssh_command("ssh host-a"), "ssh -t host-a");
        assert_eq!(attach_ssh_command("ssh -t host-a"), "ssh -t host-a");
    }

    #[test]
    fn attach_command_sets_terminal_fallback() {
        assert_eq!(
            zellij_attach_command("NeuroPlay", None),
            "TERM='xterm-256color' zellij attach 'NeuroPlay'"
        );
    }

    #[test]
    fn create_command_is_idempotent_attach_create() {
        assert_eq!(
            zellij_create_command("demo", Some("screen-256color")),
            "TERM='screen-256color' zellij attach 'demo' --create"
        );
    }

    #[test]
    fn normalizes_common_git_remote_urls() {
        assert_eq!(
            normalize_git_remote("git@github.com:user/repo.git").as_deref(),
            Some("https://github.com/user/repo")
        );
        assert_eq!(
            normalize_git_remote("ssh://git@github.com/user/repo.git").as_deref(),
            Some("https://github.com/user/repo")
        );
        assert_eq!(
            normalize_git_remote("https://github.com/user/repo.git").as_deref(),
            Some("https://github.com/user/repo")
        );
    }

    #[test]
    fn parses_scan_output_into_sessions_and_panes() {
        let raw = "##ZW-SESSION-BEGIN## demo 0\n[\n  {\n    \"id\": 0,\n    \"is_plugin\": true,\n    \"title\": \"tab-bar\"\n  },\n  {\n    \"id\": 0,\n    \"is_plugin\": false,\n    \"is_focused\": true,\n    \"title\": \"demo\",\n    \"tab_position\": 0,\n    \"tab_name\": \"Tab #1\",\n    \"pane_command\": \"claude\",\n    \"pane_cwd\": \"/repo\"\n  }\n]\n##ZW-SESSION-END##\n##ZW-SESSION-BEGIN## stale 1\n##ZW-SESSION-END##\n";
        let sessions = parse_scan_output(raw).unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].0, "demo");
        assert!(!sessions[0].1);
        assert_eq!(sessions[0].2.len(), 2);
        assert_eq!(sessions[1].0, "stale");
        assert!(sessions[1].1);
        assert!(sessions[1].2.is_empty());
    }

    #[test]
    fn scan_error_marker_becomes_an_error() {
        let raw = "##ZW-SCAN-ERROR## zellij not found in PATH\n";
        let err = parse_scan_output(raw).unwrap_err();
        assert!(err.to_string().contains("zellij not found in PATH"));
    }

    #[test]
    fn workspace_root_prefers_agent_pane_path_and_filters_plugin_panes() {
        // pane_command carries the full command line ("claude --resume"),
        // unlike tmux's bare process name; agent detection must still match.
        let raw = "##ZW-SESSION-BEGIN## demo 0\n[\n  {\"id\": 0, \"is_plugin\": true, \"title\": \"tab-bar\"},\n  {\"id\": 0, \"is_plugin\": false, \"is_focused\": false, \"pane_command\": \"claude --resume\", \"pane_cwd\": \"/repo\", \"tab_name\": \"Tab #1\", \"tab_position\": 0, \"title\": \"\"},\n  {\"id\": 1, \"is_plugin\": false, \"is_focused\": true, \"pane_command\": \"bash\", \"pane_cwd\": \"/repo/frontend\", \"tab_name\": \"Tab #1\", \"tab_position\": 0, \"title\": \"\"}\n]\n##ZW-SESSION-END##\n";
        let sessions = parse_scan_output(raw).unwrap();
        let (name, resurrectable, panes) = sessions.into_iter().next().unwrap();
        let workspace = build_workspace("host-a", &name, resurrectable, panes);
        assert_eq!(workspace.id, "host-a/demo");
        assert_eq!(workspace.panes.len(), 2);
        assert_eq!(workspace.root_path, "/repo");
        assert_eq!(workspace.agent, "claude");
    }

    #[test]
    fn process_name_takes_first_token_of_the_command_line() {
        assert_eq!(process_name(Some("claude --resume")), "claude");
        assert_eq!(process_name(Some("zsh")), "zsh");
        assert_eq!(process_name(None), "");
    }

    #[test]
    fn process_name_strips_login_shell_parens() {
        assert_eq!(process_name(Some("(bash)")), "bash");
        assert_eq!(process_name(Some("(-zsh)")), "-zsh");
    }

    #[test]
    fn identical_session_names_on_different_servers_get_distinct_ids() {
        let a = build_workspace("host-a", "api", false, Vec::new());
        let b = build_workspace("host-b", "api", false, Vec::new());
        assert_ne!(a.id, b.id);
        assert_eq!(a.id, "host-a/api");
        assert_eq!(b.id, "host-b/api");
    }

    #[test]
    fn server_command_dispatch_shape() {
        let local = server("local", true);
        let remote = server("host-a", false);
        assert!(local.local);
        assert_eq!(remote.ssh, "ssh host-a");
    }
}
