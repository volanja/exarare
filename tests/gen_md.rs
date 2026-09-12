//! Records a session and checks the runbook `exarare gen md` produces.

mod common;

use std::ffi::OsStr;
use std::fs;

use common::{record_with, which, with_exarare_on_path};

#[test]
fn generates_a_runbook() {
    let bash = which("bash").expect("bash not found");
    let watched = tempfile::tempdir().unwrap();
    let root = watched.path();
    fs::write(root.join("app.conf"), "listen 80\n").unwrap();

    let script = format!(
        "exarare note Configure the app\n\
         printf 'listen 443\\n' > {root}/app.conf\n\
         cat {root}/app.conf\n\
         bash --version\n\
         false\n\
         exit\n",
        root = root.display()
    );
    let recording = record_with(
        &bash,
        &with_exarare_on_path(&script),
        &[OsStr::new("--watch"), root.as_os_str()],
    );

    let doc = recording.exarare(&["gen", "md", &recording.session_id]);

    // Chapters, in the agreed order.
    let chapters: Vec<&str> = doc
        .lines()
        .filter(|l| l.starts_with("## "))
        .map(|l| l.trim_start_matches("## "))
        .collect();
    assert_eq!(
        chapters,
        vec![
            "1. Purpose",
            "2. Overview of the work",
            "3. Prerequisites",
            "4. Procedure",
            "5. What changed",
            "6. Verification",
            "7. Rollback",
            "Appendix A: full command log",
            "Appendix B: how this was generated",
        ],
        "{doc}"
    );

    // The human parts are left as templates.
    assert!(doc.contains("### What was done"), "{doc}");
    assert!(doc.contains("> **TODO**"), "{doc}");
    // The rollback chapter warns before it suggests anything.
    assert!(doc.contains("not a verified rollback procedure"), "{doc}");

    // The heading became a step, and the edit is shown with its diff.
    assert!(doc.contains("Configure the app"), "{doc}");
    assert!(doc.contains("-listen 80"), "{doc}");
    assert!(doc.contains("+listen 443"), "{doc}");

    // `cat` is inspection and `false` failed, so neither is an instruction,
    // but the appendix keeps both.
    let procedure = doc.split("## 4. Procedure").nth(1).unwrap();
    let procedure = procedure.split("## 5.").next().unwrap();
    assert!(!procedure.contains("cat "), "{procedure}");
    assert!(!procedure.contains("false"), "{procedure}");
    let appendix = doc.split("Appendix A").nth(1).unwrap();
    assert!(appendix.contains("false"), "{appendix}");
    assert!(appendix.contains("cat "), "{appendix}");

    // A `--version` call reads state back, so it is offered as a check.
    // `systemctl` would not do here: it does not exist on macOS.
    assert!(doc.contains("- `bash --version`"), "{doc}");
}

#[test]
fn generates_japanese_and_writes_to_a_file() {
    let bash = which("bash").expect("bash not found");
    let watched = tempfile::tempdir().unwrap();
    let recording = record_with(
        &bash,
        &with_exarare_on_path("echo hello\nexit\n"),
        &[OsStr::new("--watch"), watched.path().as_os_str()],
    );

    let out = watched.path().join("runbook.md");
    recording.exarare(&[
        "gen",
        "md",
        &recording.session_id,
        "--lang",
        "ja",
        "-o",
        out.to_str().unwrap(),
    ]);

    let doc = fs::read_to_string(&out).unwrap();
    assert!(doc.contains("## 1. 目的"), "{doc}");
    assert!(doc.contains("## 7. 切り戻し"), "{doc}");
    assert!(doc.contains("> **要記入**"), "{doc}");
    // No note was recorded, so the session is one step and says so.
    assert!(doc.contains("セッション全体"), "{doc}");
    // Recorded commands are never translated.
    assert!(doc.contains("echo hello"), "{doc}");
}
