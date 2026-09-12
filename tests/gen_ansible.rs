//! Records a session and checks the playbook `exarare gen ansible` produces.

mod common;

use std::ffi::OsStr;
use std::fs;

use common::{record_with, which, with_exarare_on_path};

#[test]
fn generates_a_playbook_and_its_files() {
    let bash = which("bash").expect("bash not found");
    let watched = tempfile::tempdir().unwrap();
    let root = watched.path();
    fs::create_dir(root.join("nginx")).unwrap();
    fs::write(root.join("nginx/nginx.conf"), "listen 80\n").unwrap();
    fs::write(root.join("legacy.conf"), "obsolete\n").unwrap();

    let script = format!(
        "exarare step Configure nginx\n\
         printf 'listen 8080\\n' > {root}/nginx/nginx.conf\n\
         chmod 600 {root}/nginx/nginx.conf\n\
         rm {root}/legacy.conf\n\
         exarare step Tidy up\n\
         mkdir -p {root}/extra\n\
         exit\n",
        root = root.display()
    );
    let recording = record_with(
        &bash,
        &with_exarare_on_path(&script),
        &[OsStr::new("--snapshot"), root.as_os_str()],
    );

    let out = tempfile::tempdir().unwrap();
    let dir = out.path().join("playbook");
    recording.exarare(&[
        "gen",
        "ansible",
        &recording.session_id,
        "-o",
        dir.to_str().unwrap(),
    ]);

    let yaml = fs::read_to_string(dir.join("playbook.yml")).unwrap();
    assert!(yaml.starts_with("---\n"), "{yaml}");
    assert!(yaml.contains("- name: \"Exarare: test\""), "{yaml}");
    assert!(yaml.contains("become: true"), "{yaml}");

    // The edited file becomes a copy task, with its content beside the playbook.
    assert!(yaml.contains("ansible.builtin.copy:"), "{yaml}");
    assert!(yaml.contains("mode: \"0600\""), "{yaml}");
    let copied = dir
        .join("files")
        .join(root.canonicalize().unwrap().strip_prefix("/").unwrap())
        .join("nginx/nginx.conf");
    assert_eq!(fs::read_to_string(&copied).unwrap(), "listen 8080\n");

    // The deleted file becomes an absent task.
    assert!(yaml.contains("state: absent"), "{yaml}");
    assert!(yaml.contains("legacy.conf"), "{yaml}");

    // `mkdir -p` matches no probe, so it is replayed behind a TODO.
    assert!(yaml.contains("TODO: make idempotent"), "{yaml}");
    assert!(yaml.contains("ansible.builtin.command: mkdir -p"), "{yaml}");
    assert!(yaml.contains("changed_when: true"), "{yaml}");

    // The step titles are carried over as comments.
    assert!(yaml.contains("# Step: Configure nginx"), "{yaml}");
    assert!(yaml.contains("# Step: Tidy up"), "{yaml}");

    // Every line stays within the limit ansible-lint enforces.
    for line in yaml.lines() {
        assert!(
            line.chars().count() <= 160,
            "line too long for ansible-lint: {line}"
        );
    }
}

#[test]
fn comments_follow_the_language_but_task_names_do_not() {
    let bash = which("bash").expect("bash not found");
    let watched = tempfile::tempdir().unwrap();
    let root = watched.path();
    fs::write(root.join("app.conf"), "a\n").unwrap();

    // The step title stays ASCII here: macOS ships bash 3.2, which mangles a
    // multibyte argument typed into a piped interactive shell. The locale
    // handling of the recording shell is a separate matter from this test.
    let script = format!(
        "exarare step Change the config\nprintf 'b\\n' > {root}/app.conf\nexit\n",
        root = root.display()
    );
    let recording = record_with(
        &bash,
        &with_exarare_on_path(&script),
        &[OsStr::new("--snapshot"), root.as_os_str()],
    );

    let out = tempfile::tempdir().unwrap();
    let dir = out.path().join("playbook");
    recording.exarare(&[
        "gen",
        "ansible",
        &recording.session_id,
        "--lang",
        "ja",
        "-o",
        dir.to_str().unwrap(),
    ]);

    let yaml = fs::read_to_string(dir.join("playbook.yml")).unwrap();
    assert!(
        yaml.contains("# 実行する前に内容を確認してください"),
        "{yaml}"
    );
    assert!(yaml.contains("# ステップ: Change the config"), "{yaml}");
    // Task names stay English so that ansible-lint's casing rule is satisfied.
    assert!(yaml.contains("- name: Copy "), "{yaml}");
}
