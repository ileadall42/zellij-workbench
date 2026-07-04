//! Simulates "multiple machines" for the remote scan/attach/doctor path.
//!
//! Each simulated host is a real, unprivileged `sshd` bound to a private
//! loopback port with its own keypair, so authentication and non-login
//! command execution over SSH are genuinely exercised. The one thing this
//! sandbox *cannot* reproduce is independent zellij session storage per
//! host: zellij keys its socket directory off the OS-assigned per-UID temp
//! directory rather than `$HOME`, so on a single dev machine every simulated
//! host ends up sharing one real session pool. Tests therefore assert what
//! is actually guaranteed here: `zw`'s own `server/session` namespacing
//! keeps two hosts' view of the same session name as two distinct
//! workspaces, and a scan costs one SSH connection per host rather than one
//! per session.

mod support;

use serde_json::Value;
use support::{
    ClientKey, Sandbox, SimulatedHost, ZellijSessionGuard, init_test_git_repo,
    loopback_ssh_available, sshd_available, zellij_available,
};

fn list_json(sandbox: &Sandbox) -> Vec<Value> {
    let output = sandbox.zw(&["list", "--all", "--json"]);
    assert!(
        output.status.success(),
        "zw list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("zw list --json output should parse")
}

fn find<'a>(workspaces: &'a [Value], id: &str) -> &'a Value {
    workspaces
        .iter()
        .find(|ws| ws["id"] == Value::String(id.to_string()))
        .unwrap_or_else(|| panic!("workspace {id} not found in {workspaces:#?}"))
}

#[test]
fn aggregates_sessions_from_two_simulated_remote_hosts() {
    if !zellij_available() || !sshd_available() || !loopback_ssh_available() {
        eprintln!("skipping: needs zellij, sshd and loopback ssh on this machine");
        return;
    }

    let client_key = ClientKey::generate();
    let host_a = SimulatedHost::spawn(&client_key);
    let host_b = SimulatedHost::spawn(&client_key);

    let repo_a = std::env::temp_dir().join(format!("zw-it-hosta-{}", std::process::id()));
    let repo_b = std::env::temp_dir().join(format!("zw-it-hostb-{}", std::process::id()));
    init_test_git_repo(&repo_a);
    init_test_git_repo(&repo_b);
    let repo_a = std::fs::canonicalize(&repo_a).unwrap();
    let repo_b = std::fs::canonicalize(&repo_b).unwrap();

    // Disjoint session names stand in for "this session only exists on this
    // machine" even though, on this sandbox, both simulated hosts can
    // technically see both sessions (see module docs above).
    let session_a = format!("hosta-web-{}", std::process::id());
    let session_b = format!("hostb-api-{}", std::process::id());
    let guard_a = ZellijSessionGuard::create_background(&session_a, &repo_a);
    let guard_b = ZellijSessionGuard::create_background(&session_b, &repo_b);

    let sandbox = Sandbox::new();
    sandbox.write_config(&format!(
        "servers:\n\
         \x20 - name: host-a\n\
         \x20   ssh: \"{ssh_a}\"\n\
         \x20   term: xterm-256color\n\
         \x20   local: false\n\
         \x20 - name: host-b\n\
         \x20   ssh: \"{ssh_b}\"\n\
         \x20   term: xterm-256color\n\
         \x20   local: false\n",
        ssh_a = host_a.ssh_command(&client_key),
        ssh_b = host_b.ssh_command(&client_key),
    ));

    let scan = sandbox.zw(&["scan"]);
    assert!(
        scan.status.success(),
        "zw scan failed: {}",
        String::from_utf8_lossy(&scan.stderr)
    );

    let workspaces = list_json(&sandbox);

    // Each configured server discovers the session that "belongs" to it...
    let on_host_a = find(&workspaces, &format!("host-a/{session_a}"));
    assert_eq!(on_host_a["server"], "host-a");
    assert_eq!(
        on_host_a["root_path"].as_str().unwrap(),
        repo_a.to_str().unwrap()
    );

    let on_host_b = find(&workspaces, &format!("host-b/{session_b}"));
    assert_eq!(on_host_b["server"], "host-b");
    assert_eq!(
        on_host_b["root_path"].as_str().unwrap(),
        repo_b.to_str().unwrap()
    );

    // ...and the id scheme keeps the same session name distinct per server,
    // which is the actual multi-host correctness guarantee `zw` provides.
    let cross_a = find(&workspaces, &format!("host-a/{session_b}"));
    let cross_b = find(&workspaces, &format!("host-b/{session_a}"));
    assert_ne!(on_host_a["id"], cross_b["id"]);
    assert_ne!(on_host_b["id"], cross_a["id"]);

    // Session/pane discovery is batched into a single SSH round trip per
    // host regardless of how many sessions live there (unlike a naive
    // one-call-per-session translation of the zellij CLI, which would not
    // scale). Git snapshotting still costs one round trip per *workspace*,
    // matching tmux-workbench's own existing (unbatched) git-scan design -
    // so the expected total is 1 (scan script) + 1 per workspace discovered
    // through that host. Both simulated hosts see both sessions here (see
    // module docs), so that is 1 + 2 = 3 for each host.
    let workspaces_via_host_a = workspaces
        .iter()
        .filter(|ws| ws["server"] == "host-a")
        .count();
    let workspaces_via_host_b = workspaces
        .iter()
        .filter(|ws| ws["server"] == "host-b")
        .count();
    assert_eq!(
        host_a.accepted_connection_count(),
        1 + workspaces_via_host_a,
        "expected one scan connection plus one git-scan connection per workspace on host-a"
    );
    assert_eq!(
        host_b.accepted_connection_count(),
        1 + workspaces_via_host_b,
        "expected one scan connection plus one git-scan connection per workspace on host-b"
    );

    let doctor = sandbox.zw(&["doctor"]);
    assert!(doctor.status.success());
    let text = String::from_utf8_lossy(&doctor.stdout);
    assert!(text.contains("server: host-a"));
    assert!(text.contains("server: host-b"));
    // `zw` always keeps an implicit "local" server around (see config.rs),
    // so doctor reports on three servers here: the auto-added local one
    // plus the two simulated remote hosts.
    assert!(text.contains("server: local"));
    assert_eq!(text.matches("ssh: ok").count(), 2);
    assert_eq!(text.matches("zellij: ok").count(), 3);

    drop(guard_a);
    drop(guard_b);
    let _ = std::fs::remove_dir_all(&repo_a);
    let _ = std::fs::remove_dir_all(&repo_b);
}
