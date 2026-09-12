//! Records a session that writes outside the snapshotted directories and checks
//! that the watcher noticed.

mod common;

use std::ffi::OsStr;
use std::fs;

use common::{record_with, which, with_exarare_on_path};

#[test]
fn reports_a_file_touched_outside_the_snapshot() {
    let bash = which("bash").expect("bash not found");
    let snapshotted = tempfile::tempdir().unwrap();
    let watched = tempfile::tempdir().unwrap();
    let elsewhere = watched.path();

    // The file lives under a watched root, not a snapshotted one, so no content
    // is recorded and only the fact that it was written can be reported.
    let script = format!(
        "exarare step Deploy the app\n\
         printf 'version: 2\\n' > {elsewhere}/app.yaml\n\
         sleep 1\n\
         exit\n",
        elsewhere = elsewhere.display()
    );
    let recording = record_with(
        &bash,
        &with_exarare_on_path(&script),
        &[
            OsStr::new("--snapshot"),
            snapshotted.path().as_os_str(),
            OsStr::new("--watch"),
            elsewhere.as_os_str(),
        ],
    );

    let report = recording.exarare(&["diff", &recording.session_id]);
    assert!(
        report
            .lines()
            .any(|l| l.starts_with("touched") && l.contains("app.yaml")),
        "the touched file is missing from the report: {report}"
    );

    let doc = recording.exarare(&["gen", "md", &recording.session_id]);
    assert!(doc.contains("### Files touched"), "{doc}");
    assert!(doc.contains("app.yaml"), "{doc}");
    // No content is claimed for it. The command that wrote the file is of
    // course shown in the procedure, so what matters is that the file does not
    // appear as a diffed change: it is not in the Files table and has no diff
    // block.
    let changed = doc.split("## 5. What changed").nth(1).unwrap();
    let changed = changed.split("### Files touched").next().unwrap();
    assert!(!changed.contains("app.yaml"), "{changed}");
    assert!(!doc.contains("```diff"), "{doc}");
    // The watched directory is named among the prerequisites.
    assert!(doc.contains("Watched for touches"), "{doc}");

    // A path the watcher saw during a step is listed under that step.
    let procedure = doc.split("## 4. Procedure").nth(1).unwrap();
    let procedure = procedure.split("## 5.").next().unwrap();
    assert!(
        procedure.contains("Files touched in this step"),
        "{procedure}"
    );

    // Nothing was left in the session for a file that was never touched.
    fs::write(snapshotted.path().join("untouched.conf"), "later\n").unwrap();
    assert!(!doc.contains("untouched.conf"), "{doc}");
}

#[test]
fn without_the_watcher_nothing_is_reported() {
    let bash = which("bash").expect("bash not found");
    let snapshotted = tempfile::tempdir().unwrap();
    let recording = record_with(
        &bash,
        &with_exarare_on_path("echo hello\nexit\n"),
        &[OsStr::new("--snapshot"), snapshotted.path().as_os_str()],
    );

    let doc = recording.exarare(&["gen", "md", &recording.session_id]);
    assert!(!doc.contains("### Files touched"), "{doc}");
    assert!(!doc.contains("Watched for touches"), "{doc}");
}
