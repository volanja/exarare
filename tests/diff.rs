//! Records a session that edits files under a watched directory and checks
//! `exarare diff`.

mod common;

use std::ffi::OsStr;
use std::fs;

use common::{record_with, which, with_exarare_on_path};

#[test]
fn reports_file_changes() {
    let bash = which("bash").expect("bash not found");
    let watched = tempfile::tempdir().unwrap();
    let root = watched.path();
    fs::write(root.join("keep.conf"), "same\n").unwrap();
    fs::write(root.join("edit.conf"), "listen 80\n").unwrap();
    fs::write(root.join("gone.conf"), "obsolete\n").unwrap();
    fs::write(root.join("chmod.conf"), "perm\n").unwrap();

    let script = format!(
        "printf 'listen 443\\n' > {root}/edit.conf\n\
         printf 'new file\\n' > {root}/added.conf\n\
         rm {root}/gone.conf\n\
         chmod 600 {root}/chmod.conf\n\
         exit\n",
        root = root.display()
    );
    let recording = record_with(
        &bash,
        &with_exarare_on_path(&script),
        &[OsStr::new("--snapshot"), root.as_os_str()],
    );

    let report = recording.exarare(&["diff", &recording.session_id]);
    let lines: Vec<&str> = report.lines().collect();

    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("modified") && l.ends_with("edit.conf")),
        "{report}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("added") && l.ends_with("added.conf")),
        "{report}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("removed") && l.ends_with("gone.conf")),
        "{report}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("permissions") && l.ends_with("chmod.conf")),
        "{report}"
    );
    assert!(
        !report.contains("keep.conf"),
        "unchanged file reported: {report}"
    );

    // The diff of the edited file shows both sides.
    assert!(report.contains("-listen 80"), "{report}");
    assert!(report.contains("+listen 443"), "{report}");
    assert!(report.contains("mode 644 -> 600"), "{report}");
}
