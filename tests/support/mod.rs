// This module is shared by several integration test binaries; each only
// uses a subset of the helpers, so the rest would otherwise warn as unused.
#![allow(dead_code)]

use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
};

/// Path to the binary built for this crate, provided by cargo for
/// integration tests without needing an extra dependency.
pub fn zw_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_zw"))
}

/// True if a working `zellij` binary is on PATH. Integration tests that
/// drive real zellij sessions skip (rather than fail) when it is absent, so
/// `cargo test` stays usable on machines without zellij installed.
pub fn zellij_available() -> bool {
    Command::new("zellij")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// True if an `ssh` client is available to drive the simulated hosts below.
/// This does not require any system sshd (on port 22 or otherwise); the
/// multi-host tests spin up their own throwaway sshd per simulated host.
pub fn loopback_ssh_available() -> bool {
    Command::new("ssh")
        .arg("-V")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// An isolated `~/.config/zw` + `~/.local/share/zw` pair for one test, so
/// tests never touch the developer's real workbench index.
pub struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("create sandbox tempdir");
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        Sandbox { dir }
    }

    pub fn config_dir(&self) -> PathBuf {
        self.dir.path().join("config")
    }

    pub fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    pub fn write_config(&self, yaml: &str) {
        std::fs::write(self.config_dir().join("config.yaml"), yaml).expect("write config.yaml");
    }

    pub fn zw(&self, args: &[&str]) -> Output {
        Command::new(zw_bin())
            .args(args)
            .env("ZW_CONFIG_DIR", self.config_dir())
            .env("ZW_DATA_DIR", self.data_dir())
            .env_remove("EDITOR")
            .output()
            .expect("run zw binary")
    }
}

/// Ensures a real zellij session is force-killed and deleted even if the
/// test body panics or an assertion fails midway.
pub struct ZellijSessionGuard {
    pub name: String,
}

impl ZellijSessionGuard {
    pub fn create_background(name: impl Into<String>, cwd: &std::path::Path) -> Self {
        let name = name.into();
        let status = Command::new("zellij")
            .args(["attach", &name, "--create-background"])
            .current_dir(cwd)
            .status()
            .expect("create background zellij session");
        assert!(status.success(), "failed to create session {name}");

        // `--create-background` returning doesn't guarantee the initial
        // pane's shell has started and reported its cwd yet (this races on
        // slower CI runners in particular — observed empty root_path on
        // GitHub's ubuntu-latest even though it never reproduced locally).
        // Wait for a real, non-plugin pane with a populated pane_cwd before
        // handing the session back so callers don't scan too early.
        let ready = wait_for(
            || terminal_pane_is_ready(&name),
            std::time::Duration::from_secs(10),
        );
        assert!(
            ready,
            "session {name} never reported a ready terminal pane in time"
        );

        ZellijSessionGuard { name }
    }
}

fn terminal_pane_is_ready(session: &str) -> bool {
    let Ok(output) = Command::new("zellij")
        .args(["--session", session, "action", "list-panes", "--all", "--json"])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let Ok(panes) = serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout) else {
        return false;
    };
    panes.iter().any(|pane| {
        pane.get("is_plugin") == Some(&serde_json::Value::Bool(false))
            && pane
                .get("pane_cwd")
                .and_then(|value| value.as_str())
                .is_some_and(|cwd| !cwd.is_empty())
    })
}

impl Drop for ZellijSessionGuard {
    fn drop(&mut self) {
        let _ = Command::new("zellij")
            .args(["delete-session", &self.name, "-f"])
            .output();
    }
}

pub fn wait_for<F: Fn() -> bool>(condition: F, timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        if condition() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// True if this machine has the binaries needed to stand up a throwaway,
/// unprivileged loopback sshd (no `sudo`, no system "Remote Login" toggle).
pub fn sshd_available() -> bool {
    Path::new("/usr/sbin/sshd").exists() && Path::new("/usr/bin/ssh-keygen").exists()
}

/// An ephemeral ed25519 keypair used to authenticate against the simulated
/// hosts below. Generated fresh per test run rather than checked into the
/// repo.
pub struct ClientKey {
    dir: tempfile::TempDir,
}

impl ClientKey {
    pub fn generate() -> Self {
        let dir = tempfile::tempdir().expect("tempdir for client key");
        let key_path = dir.path().join("id_ed25519");
        run_ok(Command::new("ssh-keygen").args([
            "-t",
            "ed25519",
            "-f",
            key_path.to_str().unwrap(),
            "-N",
            "",
            "-q",
        ]));
        ClientKey { dir }
    }

    pub fn private_key_path(&self) -> PathBuf {
        self.dir.path().join("id_ed25519")
    }

    fn public_key(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("id_ed25519.pub")).unwrap()
    }
}

/// A throwaway unprivileged `sshd` bound to 127.0.0.1 on a free high port,
/// standing in for one "remote machine" in multi-host tests. All simulated
/// hosts on this dev box still share the real user's zellij session store
/// (zellij keys its socket directory off the OS-assigned per-UID temp dir,
/// not `$HOME`), so tests give each simulated host disjoint session-name
/// prefixes rather than relying on OS-level isolation between them. What
/// *is* fully real here is the SSH transport: authentication, non-login
/// command execution, and the one-shot remote scan script.
pub struct SimulatedHost {
    child: Child,
    port: u16,
    dir: tempfile::TempDir,
}

impl SimulatedHost {
    pub fn spawn(client_key: &ClientKey) -> Self {
        let dir = tempfile::tempdir().expect("tempdir for simulated host");
        let port = free_port();

        let host_key = dir.path().join("host_key");
        run_ok(Command::new("ssh-keygen").args([
            "-t",
            "ed25519",
            "-f",
            host_key.to_str().unwrap(),
            "-N",
            "",
            "-q",
        ]));

        // Non-interactive SSH shells don't source the login-shell rc files
        // that normally put Homebrew (and thus `zellij`) on PATH, so it is
        // forced here via `environment=` the same way a real remote host's
        // shell profile would already have it configured.
        let authorized_keys = dir.path().join("authorized_keys");
        std::fs::write(
            &authorized_keys,
            format!(
                "environment=\"PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin\" {}",
                client_key.public_key()
            ),
        )
        .unwrap();
        set_mode_600(&host_key);
        set_mode_600(&authorized_keys);

        let pid_file = dir.path().join("sshd.pid");
        let sshd_config = dir.path().join("sshd_config");
        std::fs::write(
            &sshd_config,
            format!(
                "Port {port}\n\
                 ListenAddress 127.0.0.1\n\
                 HostKey {host_key}\n\
                 AuthorizedKeysFile {authorized_keys}\n\
                 PidFile {pid_file}\n\
                 PasswordAuthentication no\n\
                 KbdInteractiveAuthentication no\n\
                 UsePAM no\n\
                 StrictModes no\n\
                 PermitUserEnvironment yes\n\
                 LogLevel INFO\n",
                host_key = host_key.display(),
                authorized_keys = authorized_keys.display(),
                pid_file = pid_file.display(),
            ),
        )
        .unwrap();

        let log = std::fs::File::create(dir.path().join("sshd.log")).unwrap();
        let child = Command::new("/usr/sbin/sshd")
            .args(["-f", sshd_config.to_str().unwrap(), "-D", "-e"])
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("spawn sshd");

        let host = SimulatedHost { child, port, dir };
        let ready = wait_for(
            || std::net::TcpStream::connect(("127.0.0.1", host.port)).is_ok(),
            std::time::Duration::from_secs(5),
        );
        assert!(ready, "sshd on port {port} never started accepting connections");
        host
    }

    pub fn ssh_command(&self, client_key: &ClientKey) -> String {
        format!(
            "ssh -p {} -i {} -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o BatchMode=yes -o ConnectTimeout=5 127.0.0.1",
            self.port,
            client_key.private_key_path().display()
        )
    }

    /// Number of completed SSH authentications seen so far, used to check
    /// that one `zw scan` costs one connection per host rather than one per
    /// session (the whole point of batching the remote scan into a single
    /// script).
    pub fn accepted_connection_count(&self) -> usize {
        std::fs::read_to_string(self.dir.path().join("sshd.log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.contains("Accepted publickey"))
            .count()
    }

    /// Raw sshd log, for diagnosing auth/connection failures in CI
    /// environments this test can't be run against interactively.
    pub fn log_contents(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("sshd.log")).unwrap_or_default()
    }
}

impl Drop for SimulatedHost {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().unwrap().port()
}

fn set_mode_600(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn run_ok(command: &mut Command) {
    let status = command.status().expect("spawn command");
    assert!(status.success(), "command failed: {command:?}");
}

pub fn init_test_git_repo(path: &std::path::Path) {
    std::fs::create_dir_all(path).unwrap();
    let run = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(path)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "zw-test@example.com"]);
    run(&["config", "user.name", "zw-test"]);
    std::fs::write(path.join("README.md"), "zw integration test fixture\n").unwrap();
    run(&["add", "README.md"]);
    run(&["commit", "-q", "-m", "initial commit"]);
}
