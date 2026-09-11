//! Drives a real recording shell with commands on stdin and checks events.jsonl.

mod common;

use common::{Recorded, Recording, record, which, with_exarare_on_path};

const SCRIPT: &str =
    "echo hello\nfalse\ncd /tmp && ls >/dev/null\nexarare note install step\nexit\n";

fn expected() -> Vec<Recorded> {
    [
        ("echo hello", Some(0)),
        ("false", Some(1)),
        ("cd /tmp && ls >/dev/null", Some(0)),
        ("exarare note install step", Some(0)),
        ("exit", None),
    ]
    .into_iter()
    .map(|(cmd, exit_code)| Recorded {
        cmd: cmd.into(),
        exit_code,
    })
    .collect()
}

fn check(recording: Recording) {
    // The PATH line prepended by the harness is recorded first.
    assert!(recording.commands[0].cmd.starts_with("PATH="));
    assert_eq!(recording.commands[1..], expected());
    assert_eq!(recording.notes, vec!["install step".to_string()]);
}

#[test]
fn records_bash() {
    let bash = which("bash").expect("bash not found");
    check(record(&bash, &with_exarare_on_path(SCRIPT)));
}

#[test]
fn records_zsh() {
    let Some(zsh) = which("zsh") else {
        eprintln!("zsh not found, skipping");
        return;
    };
    check(record(&zsh, &with_exarare_on_path(SCRIPT)));
}
