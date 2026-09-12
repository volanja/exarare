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
        "exarare step Configure the app\n\
         printf 'listen 443\\n' > {root}/app.conf\n\
         exarare note reload is required afterwards\n\
         chmod 600 {root}/app.conf\n\
         cat {root}/app.conf\n\
         bash --version\n\
         false\n\
         exit\n",
        root = root.display()
    );
    let recording = record_with(
        &bash,
        &with_exarare_on_path(&script),
        &[OsStr::new("--snapshot"), root.as_os_str()],
    );
    assert_eq!(recording.steps, vec!["Configure the app".to_string()]);
    assert_eq!(
        recording.notes,
        vec!["reload is required afterwards".to_string()]
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
            "Appendix C: how this was generated",
        ],
        "{doc}"
    );

    // The human parts are left as templates.
    assert!(doc.contains("### What was done"), "{doc}");
    assert!(doc.contains("> **TODO**"), "{doc}");
    // The rollback chapter warns before it suggests anything.
    assert!(doc.contains("not a verified rollback procedure"), "{doc}");

    // `exarare step` became the section heading, and the edit is shown with its diff.
    assert!(doc.contains("Configure the app"), "{doc}");
    assert!(doc.contains("-listen 80"), "{doc}");
    assert!(doc.contains("+listen 443"), "{doc}");

    let procedure = doc.split("## 4. Procedure").nth(1).unwrap();
    let procedure = procedure.split("## 5.").next().unwrap();

    // `exarare note` is a remark inside the step, placed where it was recorded.
    let note_at = procedure
        .find("> **Note**: reload is required afterwards")
        .unwrap_or_else(|| panic!("note missing: {procedure}"));
    let printf_at = procedure.find("printf 'listen 443").unwrap();
    let chmod_at = procedure.find("chmod 600").unwrap();
    assert!(printf_at < note_at && note_at < chmod_at, "{procedure}");

    // `cat` is inspection, `bash --version` is a check and `false` failed, so
    // none of them is an instruction, but the appendix keeps all of them.
    assert!(!procedure.contains("cat "), "{procedure}");
    assert!(!procedure.contains("false"), "{procedure}");
    assert!(!procedure.contains("bash --version"), "{procedure}");
    let appendix = doc.split("Appendix A").nth(1).unwrap();
    assert!(appendix.contains("false"), "{appendix}");
    assert!(appendix.contains("cat "), "{appendix}");

    // A `--version` call reads state back, so it is offered as a check.
    // `systemctl` would not do here: it does not exist on macOS.
    assert!(doc.contains("- `bash --version`"), "{doc}");
}

#[test]
fn draws_the_overview_as_a_flowchart_on_request() {
    let bash = which("bash").expect("bash not found");
    let watched = tempfile::tempdir().unwrap();
    let script = "exarare step Install the package\necho installing\nexit\n";
    let recording = record_with(
        &bash,
        &with_exarare_on_path(script),
        &[OsStr::new("--snapshot"), watched.path().as_os_str()],
    );

    let plain = recording.exarare(&["gen", "md", &recording.session_id]);
    assert!(!plain.contains("```mermaid"), "{plain}");

    let drawn = recording.exarare(&["gen", "md", &recording.session_id, "--mermaid"]);
    assert!(drawn.contains("```mermaid"), "{drawn}");
    assert!(drawn.contains("flowchart TD"), "{drawn}");
    assert!(drawn.contains("Install the package"), "{drawn}");
    // The steps are linked in the order the work happened.
    assert!(drawn.contains("step1 --> step2"), "{drawn}");
}

#[test]
fn generates_japanese_and_writes_to_a_file() {
    let bash = which("bash").expect("bash not found");
    let watched = tempfile::tempdir().unwrap();
    let recording = record_with(
        &bash,
        &with_exarare_on_path("echo hello\nexit\n"),
        &[OsStr::new("--snapshot"), watched.path().as_os_str()],
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
    // No step was recorded, so the session is one step and says so.
    assert!(doc.contains("セッション全体"), "{doc}");
    assert!(doc.contains("`exarare step"), "{doc}");
    // Recorded commands are never translated.
    assert!(doc.contains("echo hello"), "{doc}");
}
