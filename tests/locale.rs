//! Measures how the recording shell handles a non-ASCII step title.
//!
//! bash 3.2, which macOS still ships, passes a mangled argument when the locale
//! says the encoding is single-byte (see issue #22), so it is skipped here with
//! its version reported. Newer shells — the bash 4.4, 5.1 and 5.2 of EL 8, 9
//! and 10, and zsh — are expected to get it right.

mod common;

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use common::{record_with, which, with_exarare_on_path};

const TITLE: &str = "設定変更";

fn version_of(shell: &Path) -> String {
    Command::new(shell)
        .arg("--version")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.lines().next().map(str::to_string))
        .unwrap_or_default()
}

/// bash 3.x cannot be relied on here, and macOS ships 3.2.
fn too_old(version: &str) -> bool {
    version.contains("version 3.")
}

fn check(shell: &Path) {
    let version = version_of(shell);
    if too_old(&version) {
        eprintln!("skipping {}: {version}", shell.display());
        return;
    }
    let watched = tempfile::tempdir().unwrap();
    let script = format!("exarare step {TITLE}\necho recorded\nexit\n");
    let recording = record_with(
        shell,
        &with_exarare_on_path(&script),
        &[OsStr::new("--watch"), watched.path().as_os_str()],
    );

    assert_eq!(
        recording.steps,
        vec![TITLE.to_string()],
        "{} ({version}) did not pass the title through: {:?}",
        shell.display(),
        recording.steps
    );

    // The title must survive into the generated documents as well.
    let doc = recording.exarare(&["gen", "md", &recording.session_id]);
    assert!(doc.contains(TITLE), "{doc}");
}

#[test]
fn bash_records_a_multibyte_step_title() {
    let bash = which("bash").expect("bash not found");
    check(&bash);
}

#[test]
fn zsh_records_a_multibyte_step_title() {
    let Some(zsh) = which("zsh") else {
        eprintln!("zsh not found, skipping");
        return;
    };
    check(&zsh);
}
