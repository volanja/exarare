//! Drives a real recording shell with commands on stdin and checks events.jsonl.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(name))
            .find(|p| p.is_file())
    })
}

/// Returns (command, exit code) pairs from the only recorded session.
fn record(shell: &Path, script: &str) -> Vec<(String, Option<i64>)> {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let data = tmp.path().join("data");

    let mut child = Command::new(env!("CARGO_BIN_EXE_exarare"))
        .args(["start", "--name", "test", "--shell"])
        .arg(shell)
        .env("HOME", &home)
        .env("EXARARE_HOME", &data)
        .env_remove("EXARARE_SESSION_DIR")
        .env_remove("ZDOTDIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());

    let sessions: Vec<_> = std::fs::read_dir(data.join("sessions"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(sessions.len(), 1);
    let meta: Value =
        serde_json::from_str(&std::fs::read_to_string(sessions[0].join("meta.json")).unwrap())
            .unwrap();
    assert!(meta["ended_at"].is_string());

    let events: Vec<Value> = std::fs::read_to_string(sessions[0].join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(events.first().unwrap()["kind"], "session_start");
    assert_eq!(events.last().unwrap()["kind"], "session_end");

    let mut commands: Vec<(String, Option<i64>)> = Vec::new();
    let mut last_cmd = None;
    for ev in &events {
        match ev["kind"].as_str().unwrap() {
            "cmd_start" => {
                last_cmd = Some(commands.len());
                commands.push((ev["cmd"].as_str().unwrap().to_string(), None));
            }
            "cmd_end" => commands[last_cmd.unwrap()].1 = ev["exit_code"].as_i64(),
            "note" => commands.push((format!("# {}", ev["text"].as_str().unwrap()), Some(0))),
            _ => {}
        }
    }
    commands
}

const SCRIPT: &str =
    "echo hello\nfalse\ncd /tmp && ls >/dev/null\nexarare note install step\nexit\n";

fn expected() -> Vec<(String, Option<i64>)> {
    vec![
        ("echo hello".into(), Some(0)),
        ("false".into(), Some(1)),
        ("cd /tmp && ls >/dev/null".into(), Some(0)),
        ("exarare note install step".into(), Some(0)),
        ("exit".into(), None),
    ]
}

/// `exarare note` appends its own event while the command is running.
fn normalize(mut got: Vec<(String, Option<i64>)>) -> Vec<(String, Option<i64>)> {
    got.retain(|(c, _)| !c.starts_with("# "));
    got
}

fn with_exarare_on_path(script: &str) -> String {
    let bin_dir = Path::new(env!("CARGO_BIN_EXE_exarare")).parent().unwrap();
    format!("PATH={}:$PATH\n{script}", bin_dir.display())
}

#[test]
fn records_bash() {
    let bash = which("bash").expect("bash not found");
    let got = record(&bash, &with_exarare_on_path(SCRIPT));
    // The PATH line itself is recorded first.
    assert!(got[0].0.starts_with("PATH="));
    assert!(got.contains(&("# install step".into(), Some(0))));
    assert_eq!(normalize(got[1..].to_vec()), expected());
}

#[test]
fn records_zsh() {
    let Some(zsh) = which("zsh") else {
        eprintln!("zsh not found, skipping");
        return;
    };
    let got = record(&zsh, &with_exarare_on_path(SCRIPT));
    assert!(got[0].0.starts_with("PATH="));
    assert!(got.contains(&("# install step".into(), Some(0))));
    assert_eq!(normalize(got[1..].to_vec()), expected());
}
