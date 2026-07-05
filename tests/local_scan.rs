//! End-to-end test driving the real `zellij` binary and the compiled `zw`
//! binary together: create a real background session, scan it, and check
//! the resulting index through the CLI's own JSON output.

mod support;

use std::time::Duration;

use serde_json::Value;
use support::{Sandbox, ZellijSessionGuard, init_test_git_repo, wait_for, zellij_available};

fn local_config_yaml() -> &'static str {
    "servers:\n  - name: local\n    ssh: \"\"\n    term: xterm-256color\n    local: true\n"
}

fn list_json(sandbox: &Sandbox) -> Vec<Value> {
    let output = sandbox.zw(&["list", "--all", "--json"]);
    assert!(
        output.status.success(),
        "zw list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("zw list --json output should parse")
}

#[test]
fn scans_a_real_local_session_end_to_end() {
    if !zellij_available() {
        eprintln!("skipping: zellij is not installed on this machine");
        return;
    }

    let sandbox = Sandbox::new();
    sandbox.write_config(local_config_yaml());

    let repo_dir = std::env::temp_dir().join(format!("zw-it-repo-{}", std::process::id()));
    init_test_git_repo(&repo_dir);
    let repo_dir = std::fs::canonicalize(&repo_dir).unwrap();

    let session_name = format!("zw-it-local-{}", std::process::id());
    let guard = ZellijSessionGuard::create_background(&session_name, &repo_dir);

    let scan = sandbox.zw(&["scan"]);
    assert!(
        scan.status.success(),
        "zw scan failed: {}",
        String::from_utf8_lossy(&scan.stderr)
    );

    let workspaces = list_json(&sandbox);
    let workspace = workspaces
        .iter()
        .find(|ws| ws["session"] == Value::String(session_name.clone()))
        .unwrap_or_else(|| panic!("session {session_name} not found in {workspaces:#?}"));

    assert_eq!(workspace["server"], "local");
    assert_eq!(workspace["presence"], "seen");
    assert_eq!(workspace["resurrectable"], false);
    assert_eq!(
        workspace["root_path"].as_str().unwrap(),
        repo_dir.to_str().unwrap()
    );
    assert!(
        !workspace["panes"].as_array().unwrap().is_empty(),
        "expected at least one terminal pane"
    );
    let git = &workspace["git"];
    assert_eq!(git["branch"], "main");
    assert_eq!(git["dirty"], false);

    // Dirty the tree and confirm a rescan picks it up.
    std::fs::write(repo_dir.join("scratch.txt"), "dirty\n").unwrap();
    let rescan = sandbox.zw(&["scan"]);
    assert!(rescan.status.success());
    let workspaces = list_json(&sandbox);
    let workspace = workspaces
        .iter()
        .find(|ws| ws["session"] == Value::String(session_name.clone()))
        .unwrap();
    assert_eq!(workspace["git"]["dirty"], true);

    // User metadata must survive a rescan.
    let id = workspace["id"].as_str().unwrap().to_string();
    assert!(sandbox.zw(&["alias", &id, "demo"]).status.success());
    assert!(sandbox.zw(&["note", &id, "uses uv"]).status.success());
    assert!(
        sandbox
            .zw(&["tags", &id, "work", "backend"])
            .status
            .success()
    );
    assert!(sandbox.zw(&["scan"]).status.success());

    let workspaces = list_json(&sandbox);
    let workspace = workspaces.iter().find(|ws| ws["id"] == id).unwrap();
    assert_eq!(workspace["alias"], "demo");
    assert_eq!(workspace["note"], "uses uv");
    assert_eq!(
        workspace["tags"].as_array().unwrap(),
        &[
            Value::String("work".into()),
            Value::String("backend".into())
        ]
    );

    // Killing the session should flip presence to missing while keeping the
    // alias/note/tags the user set above. `delete-session -f` on this zellij
    // build removes the session outright (no EXITED/resurrectable entry is
    // left behind), and it can report a non-zero exit even though the kill
    // itself succeeded, so only the eventual absence from `list-sessions` is
    // asserted here.
    let _ = std::process::Command::new("zellij")
        .args(["delete-session", &session_name, "-f"])
        .output();
    let gone = wait_for(
        || {
            std::process::Command::new("zellij")
                .args(["list-sessions", "--short"])
                .output()
                .map(|out| !String::from_utf8_lossy(&out.stdout).contains(&session_name))
                .unwrap_or(false)
        },
        Duration::from_secs(5),
    );
    assert!(gone, "session {session_name} still present after delete");

    assert!(sandbox.zw(&["scan"]).status.success());
    let workspaces = list_json(&sandbox);
    let workspace = workspaces.iter().find(|ws| ws["id"] == id).unwrap();
    assert_eq!(workspace["presence"], "missing");
    assert_eq!(workspace["alias"], "demo");
    assert_eq!(workspace["note"], "uses uv");

    drop(guard);
    let _ = std::fs::remove_dir_all(&repo_dir);
}

#[test]
fn doctor_reports_local_zellij_availability() {
    if !zellij_available() {
        eprintln!("skipping: zellij is not installed on this machine");
        return;
    }

    let sandbox = Sandbox::new();
    sandbox.write_config(local_config_yaml());
    let doctor = sandbox.zw(&["doctor"]);
    assert!(doctor.status.success());
    let text = String::from_utf8_lossy(&doctor.stdout);
    assert!(text.contains("connection: local"));
    assert!(text.contains("zellij: ok"));
}
