//! Helpers for driving a real recording shell from tests.
//!
//! Each test binary includes this module, so not every helper is used by all of them.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

pub fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(name))
            .find(|p| p.is_file())
    })
}

pub fn exarare_bin() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_exarare"))
}

/// Prefixes the script with a PATH that makes `exarare` callable from the shell.
pub fn with_exarare_on_path(script: &str) -> String {
    let bin_dir = exarare_bin().parent().unwrap();
    format!("PATH={}:$PATH\n{script}", bin_dir.display())
}

/// A command line recorded in a session, with the exit code of its `cmd_end`.
#[derive(Debug, PartialEq)]
pub struct Recorded {
    pub cmd: String,
    pub exit_code: Option<i64>,
}

pub struct Session {
    pub commands: Vec<Recorded>,
    pub notes: Vec<String>,
}

impl Session {
    pub fn command_lines(&self) -> Vec<&str> {
        self.commands.iter().map(|c| c.cmd.as_str()).collect()
    }

    pub fn find(&self, cmd: &str) -> Option<&Recorded> {
        self.commands.iter().find(|c| c.cmd == cmd)
    }
}

/// Runs `exarare start` with `script` on stdin and returns what was recorded.
pub fn record(shell: &Path, script: &str) -> Session {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let data = tmp.path().join("data");

    let mut child = Command::new(exarare_bin())
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
    assert!(meta["ended_at"].is_string(), "session was not finished");

    let events: Vec<Value> = std::fs::read_to_string(sessions[0].join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(events.first().unwrap()["kind"], "session_start");
    assert_eq!(events.last().unwrap()["kind"], "session_end");

    let mut commands: Vec<Recorded> = Vec::new();
    let mut notes = Vec::new();
    let mut last_cmd = None;
    for ev in &events {
        match ev["kind"].as_str().unwrap() {
            "cmd_start" => {
                last_cmd = Some(commands.len());
                commands.push(Recorded {
                    cmd: ev["cmd"].as_str().unwrap().to_string(),
                    exit_code: None,
                });
            }
            "cmd_end" => commands[last_cmd.unwrap()].exit_code = ev["exit_code"].as_i64(),
            "note" => notes.push(ev["text"].as_str().unwrap().to_string()),
            _ => {}
        }
    }
    Session { commands, notes }
}
