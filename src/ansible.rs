//! Renders a recorded session as an Ansible playbook.
//!
//! Task names stay English: ansible-lint expects them to start with a capital
//! letter, and the playbook is read by a machine as much as by a person. The
//! comments follow `--lang`, because they are what a reviewer reads.
//!
//! Only what the probes observed becomes a task. A command that changed
//! something the probes cannot express is replayed verbatim behind a TODO,
//! rather than being guessed at or dropped.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::diff::Change;
use crate::i18n::Messages;
use crate::packages::Package;
use crate::session::Meta;
use crate::snapshot::{Content, Entry, blob_text};
use crate::state;
use crate::step::{Runbook, Step};

pub struct Input<'a> {
    pub meta: &'a Meta,
    pub runbook: &'a Runbook,
    pub blobs: &'a Path,
    pub version: &'a str,
}

pub struct Playbook {
    pub yaml: String,
    /// Content to write under `files/`, as (relative path, content).
    pub files: Vec<(PathBuf, String)>,
    /// What a human must resolve before the playbook can be trusted.
    pub warnings: Vec<String>,
}

/// Indentation: a play at column 0, its keys at 2, tasks at 4, module
/// arguments at 8.
const TASK: &str = "    ";
const KEY: &str = "      ";
const ARG: &str = "        ";
const ITEM: &str = "          ";

/// Comment lines are wrapped: ansible-lint rejects a line over 160
/// characters, and a wall of text is not read anyway.
const COMMENT_WIDTH: usize = 96;

fn wrap(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            if !current.is_empty()
                && current.chars().count() + 1 + word.chars().count() > COMMENT_WIDTH
            {
                lines.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        lines.push(current);
    }
    lines
}

/// Quotes a scalar when YAML would otherwise misread it.
fn scalar(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value
            .chars()
            .next()
            .is_some_and(|c| "!&*?|>%@`{[,#\"'-".contains(c))
        || value.contains(": ")
        || value.ends_with(':')
        || value.contains(" #")
        || value.contains(['"', '\\'])
        || value.parse::<f64>().is_ok()
        || matches!(
            value.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~"
        );
    if needs_quotes {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

/// `files/etc/nginx/nginx.conf` for `/etc/nginx/nginx.conf`.
fn files_path(path: &Path) -> PathBuf {
    let relative = path.to_string_lossy().trim_start_matches('/').to_string();
    PathBuf::from(relative)
}

fn mode_of(entry: &Entry) -> String {
    format!("0{:o}", entry.mode)
}

struct Builder<'a> {
    out: String,
    files: Vec<(PathBuf, String)>,
    warnings: Vec<String>,
    blobs: &'a Path,
    msg: &'a Messages,
}

impl<'a> Builder<'a> {
    fn comment(&mut self, text: &str) {
        for line in wrap(text) {
            let _ = writeln!(self.out, "{TASK}# {line}");
        }
    }

    fn task(&mut self, name: &str, module: &str) {
        let _ = writeln!(self.out, "{TASK}- name: {}", scalar(name));
        let _ = writeln!(self.out, "{KEY}{module}:");
    }

    fn arg(&mut self, key: &str, value: &str) {
        let _ = writeln!(self.out, "{ARG}{key}: {value}");
    }

    fn packages(&mut self, packages: &[Package]) {
        if packages.is_empty() {
            return;
        }
        self.task("Install packages", "ansible.builtin.dnf");
        let _ = writeln!(self.out, "{ARG}name:");
        for package in packages {
            let _ = writeln!(self.out, "{ITEM}- {}", scalar(&package.name));
        }
        self.arg("state", "present");
        self.out.push('\n');
    }

    fn file(&mut self, change: &Change) {
        let path = change.path();
        let display = path.display().to_string();
        match change {
            Change::Removed(_) => {
                self.task(&format!("Remove {display}"), "ansible.builtin.file");
                self.arg("path", &scalar(&display));
                self.arg("state", "absent");
                self.out.push('\n');
            }
            Change::MetadataChanged { after, .. } => {
                self.owner_comment(after);
                self.task(
                    &format!("Set permissions on {display}"),
                    "ansible.builtin.file",
                );
                self.arg("path", &scalar(&display));
                self.arg("mode", &format!("\"{}\"", mode_of(after)));
                self.out.push('\n');
            }
            Change::Added(entry) | Change::Modified { after: entry, .. } => {
                if matches!(entry.content, Content::Symlink { .. }) {
                    self.symlink(entry);
                    return;
                }
                let Some(content) = blob_text(self.blobs, entry) else {
                    // Masked, binary or oversized: the content is genuinely not
                    // available, so the playbook must not pretend otherwise.
                    let note = self.msg.todo_content_missing(&display);
                    self.comment(&note);
                    self.warnings.push(note);
                    self.out.push('\n');
                    return;
                };
                let src = files_path(path);
                self.files.push((src.clone(), content));
                // The comment comes first: after the task it would read as a
                // remark about whatever follows.
                self.owner_comment(entry);
                self.task(&format!("Copy {display}"), "ansible.builtin.copy");
                self.arg("src", &scalar(&format!("files/{}", src.display())));
                self.arg("dest", &scalar(&display));
                self.arg("mode", &format!("\"{}\"", mode_of(entry)));
                self.arg("backup", "true");
                self.out.push('\n');
            }
        }
    }

    fn symlink(&mut self, entry: &Entry) {
        let Content::Symlink { target } = &entry.content else {
            return;
        };
        let display = entry.path.display().to_string();
        self.task(&format!("Link {display}"), "ansible.builtin.file");
        self.arg("src", &scalar(&target.display().to_string()));
        self.arg("dest", &scalar(&display));
        self.arg("state", "link");
        self.out.push('\n');
    }

    /// The recorded owner is written as a comment: `owner` takes a name, and
    /// the snapshot holds numeric ids, which may belong to a different account
    /// on the host this playbook runs against.
    fn owner_comment(&mut self, entry: &Entry) {
        let note = self.msg.recorded_owner(entry.uid, entry.gid);
        self.comment(&note);
    }

    fn units(&mut self, diff: &state::Diff, names: &[String]) {
        for name in names {
            if !diff.units_enabled.contains(name) && !diff.services_started.contains(name) {
                continue;
            }
            self.task(&format!("Enable {name}"), "ansible.builtin.systemd");
            self.arg("name", &scalar(name));
            if diff.units_enabled.contains(name) {
                self.arg("enabled", "true");
            }
            if diff.services_started.contains(name) {
                self.arg("state", "started");
            }
            self.out.push('\n');
        }
    }

    fn firewall(&mut self, diff: &state::Diff) {
        for service in &diff.firewall_services_added {
            self.task(
                &format!("Open firewall service {service}"),
                "ansible.posix.firewalld",
            );
            self.arg("service", &scalar(service));
            self.firewall_tail("enabled");
        }
        for service in &diff.firewall_services_removed {
            self.task(
                &format!("Close firewall service {service}"),
                "ansible.posix.firewalld",
            );
            self.arg("service", &scalar(service));
            self.firewall_tail("disabled");
        }
        for port in &diff.firewall_ports_added {
            self.task(&format!("Open port {port}"), "ansible.posix.firewalld");
            self.arg("port", &scalar(port));
            self.firewall_tail("enabled");
        }
        for port in &diff.firewall_ports_removed {
            self.task(&format!("Close port {port}"), "ansible.posix.firewalld");
            self.arg("port", &scalar(port));
            self.firewall_tail("disabled");
        }
    }

    fn firewall_tail(&mut self, state: &str) {
        self.arg("permanent", "true");
        self.arg("immediate", "true");
        self.arg("state", state);
        self.out.push('\n');
    }

    fn accounts(&mut self, diff: &state::Diff) {
        for group in &diff.groups_added {
            self.task(
                &format!("Create group {}", group.name),
                "ansible.builtin.group",
            );
            self.arg("name", &scalar(&group.name));
            self.arg("gid", &group.gid.to_string());
            self.arg("state", "present");
            self.out.push('\n');
        }
        for user in &diff.users_added {
            self.task(
                &format!("Create user {}", user.name),
                "ansible.builtin.user",
            );
            self.arg("name", &scalar(&user.name));
            self.arg("uid", &user.uid.to_string());
            self.arg("home", &scalar(&user.home));
            self.arg("shell", &scalar(&user.shell));
            self.arg("state", "present");
            self.out.push('\n');
        }
        for user in &diff.users_removed {
            self.task(
                &format!("Remove user {}", user.name),
                "ansible.builtin.user",
            );
            self.arg("name", &scalar(&user.name));
            self.arg("state", "absent");
            self.out.push('\n');
        }
        for group in &diff.groups_removed {
            self.task(
                &format!("Remove group {}", group.name),
                "ansible.builtin.group",
            );
            self.arg("name", &scalar(&group.name));
            self.arg("state", "absent");
            self.out.push('\n');
        }
    }

    /// A command whose effect no probe could express. It is replayed as it was
    /// typed, which is not idempotent, so it carries a TODO.
    fn command(&mut self, cmd: &str) {
        let note = self.msg.todo_idempotent();
        self.comment(note);
        self.warnings.push(format!("{note}: {cmd}"));
        let needs_shell = cmd.contains(['|', '>', '<', '&', ';', '$', '*', '`']);
        let module = if needs_shell {
            "ansible.builtin.shell"
        } else {
            "ansible.builtin.command"
        };
        let _ = writeln!(self.out, "{TASK}- name: {}", scalar(&summarize(cmd)));
        let _ = writeln!(self.out, "{KEY}{module}: {}", scalar(cmd));
        // The record cannot say whether a replay would change anything, and
        // claiming "never changed" would be a lie.
        let _ = writeln!(self.out, "{KEY}changed_when: true");
        self.out.push('\n');
    }
}

/// A task name for a command: the first words, without arguments that would
/// make the name unreadable.
fn summarize(cmd: &str) -> String {
    let short: String = cmd.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
    let mut name = format!("Run {short}");
    if short.len() < cmd.trim().len() {
        name.push_str(" ...");
    }
    name
}

/// Whether the command is already covered by a task generated from a probe.
fn covered(cmd: &str, step: &Step, changes: &[&Change]) -> bool {
    step.packages
        .iter()
        .any(|p| crate::step::names_item(cmd, &p.name))
        || step
            .state
            .iter()
            .any(|name| crate::step::names_item(cmd, name))
        || changes
            .iter()
            .any(|change| crate::step::mentions_path(cmd, change.path()))
}

pub fn render(input: &Input, msg: &Messages) -> Playbook {
    let runbook = input.runbook;
    let mut builder = Builder {
        out: String::new(),
        files: Vec::new(),
        warnings: Vec::new(),
        blobs: input.blobs,
        msg,
    };

    let mut header = String::from("---\n");
    let intro = format!(
        "{}\n{}",
        msg.playbook_header(&input.meta.id, input.version),
        msg.playbook_review()
    );
    for line in wrap(&intro) {
        let _ = writeln!(header, "# {line}");
    }
    // ansible-lint requires a name to start with a capital letter, and the
    // session name is the operator's text, so it is prefixed rather than
    // rewritten.
    let name = format!(
        "Exarare: {}",
        input
            .meta
            .name
            .clone()
            .unwrap_or_else(|| input.meta.id.clone())
    );
    let _ = writeln!(header, "\n- name: {}", scalar(&name));
    let _ = writeln!(header, "  hosts: all");
    let _ = writeln!(header, "  become: true");
    let _ = writeln!(header, "  tasks:");

    for step in &runbook.steps {
        let step_changes: Vec<&Change> = step.files.iter().collect();
        let has_work = !step.packages.is_empty()
            || !step.files.is_empty()
            || !step.state.is_empty()
            || step
                .commands()
                .any(|run| !covered(&run.cmd, step, &step_changes));
        if !has_work {
            continue;
        }
        if let Some(title) = &step.title {
            let comment = msg.playbook_step(title);
            builder.comment(&comment);
        }
        builder.packages(&step.packages);
        for change in &step.files {
            builder.file(change);
        }
        let state = runbook.state.clone();
        builder.units(&state, &step.state);
        for run in step.commands() {
            if !covered(&run.cmd, step, &step_changes) {
                builder.command(&run.cmd);
            }
        }
    }

    // Changes and packages no step claimed, plus firewall rules and accounts,
    // which are session-wide.
    let unattributed: Vec<&Change> = runbook.other_files.iter().collect();
    let has_rest = !runbook.other_packages.is_empty()
        || !unattributed.is_empty()
        || !runbook.state.firewall_services_added.is_empty()
        || !runbook.state.firewall_services_removed.is_empty()
        || !runbook.state.firewall_ports_added.is_empty()
        || !runbook.state.firewall_ports_removed.is_empty()
        || !runbook.state.users_added.is_empty()
        || !runbook.state.users_removed.is_empty()
        || !runbook.state.groups_added.is_empty()
        || !runbook.state.groups_removed.is_empty();
    if has_rest {
        builder.comment(msg.playbook_unattributed());
        builder.packages(&runbook.other_packages);
        for change in &runbook.other_files {
            builder.file(change);
        }
        let state = runbook.state.clone();
        builder.firewall(&state);
        builder.accounts(&state);
    }

    let mut yaml = header;
    if builder.out.trim().is_empty() {
        // A play needs at least one task to be valid YAML for Ansible.
        let _ = writeln!(yaml, "{TASK}- name: Nothing was recorded to replay");
        let _ = writeln!(yaml, "{KEY}ansible.builtin.debug:");
        let _ = writeln!(yaml, "{ARG}msg: {}", scalar(msg.playbook_empty()));
    } else {
        yaml.push_str(&builder.out);
    }
    Playbook {
        yaml: yaml.trim_end().to_string() + "\n",
        files: builder.files,
        warnings: builder.warnings,
    }
}

/// Writes `playbook.yml` and the `files/` tree into `dir`.
pub fn write(playbook: &Playbook, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join("playbook.yml");
    std::fs::write(&path, &playbook.yaml)
        .with_context(|| format!("failed to write {}", path.display()))?;
    for (relative, content) in &playbook.files {
        let target = dir.join("files").join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, content)
            .with_context(|| format!("failed to write {}", target.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Locale;
    use crate::snapshot::Content;
    use crate::step;
    use time::OffsetDateTime;

    fn meta() -> Meta {
        Meta {
            id: "20260912-101500-0001".into(),
            name: Some("web01 nginx setup".into()),
            hostname: Some("web01".into()),
            user: Some("root".into()),
            shell: "/bin/bash".into(),
            snapshot_roots: crate::session::default_snapshot_roots(),
            watch_roots: Vec::new(),
            started_at: OffsetDateTime::UNIX_EPOCH,
            ended_at: Some(OffsetDateTime::UNIX_EPOCH),
            shell_pid: Some(1),
        }
    }

    fn entry(path: &str, mode: u32) -> Entry {
        Entry {
            path: PathBuf::from(path),
            content: Content::File {
                size: 1,
                hash: "hash".into(),
                blob: false,
                masked: false,
            },
            mode,
            uid: 0,
            gid: 0,
        }
    }

    fn rendered(runbook: &Runbook, blobs: &Path) -> Playbook {
        let meta = meta();
        render(
            &Input {
                meta: &meta,
                runbook,
                blobs,
                version: "0.1.0",
            },
            &Locale::En.messages(),
        )
    }

    #[test]
    fn quotes_only_what_yaml_would_misread() {
        assert_eq!(scalar("nginx"), "nginx");
        assert_eq!(scalar("/etc/hosts"), "/etc/hosts");
        assert_eq!(scalar("true"), "\"true\"");
        assert_eq!(scalar("8080"), "\"8080\"");
        assert_eq!(scalar("- dash"), "\"- dash\"");
        assert_eq!(scalar("say \"hi\""), "\"say \\\"hi\\\"\"");
    }

    #[test]
    fn a_command_with_a_redirect_uses_shell() {
        let events = [
            crate::event::Event {
                ts: OffsetDateTime::UNIX_EPOCH,
                kind: crate::event::EventKind::CmdStart {
                    cmd: "printf 'x\\n' > /opt/app/config".into(),
                    cwd: "/root".into(),
                },
            },
            crate::event::Event {
                ts: OffsetDateTime::UNIX_EPOCH,
                kind: crate::event::EventKind::CmdEnd { exit_code: 0 },
            },
        ];
        let runbook = step::build(&events, &[]);
        let playbook = rendered(&runbook, Path::new("/nonexistent"));

        assert!(
            playbook.yaml.contains("ansible.builtin.shell:"),
            "{}",
            playbook.yaml
        );
        assert!(
            playbook.yaml.contains("changed_when: true"),
            "{}",
            playbook.yaml
        );
        // The reviewer is told, rather than the playbook pretending to be idempotent.
        assert_eq!(playbook.warnings.len(), 1);
        assert!(playbook.warnings[0].contains("idempotent"));
    }

    #[test]
    fn an_empty_session_still_produces_a_valid_play() {
        let runbook = step::build(&[], &[]);
        let playbook = rendered(&runbook, Path::new("/nonexistent"));
        assert!(playbook.yaml.starts_with("---\n"));
        assert!(playbook.yaml.contains("ansible.builtin.debug:"));
        assert!(playbook.files.is_empty());
    }

    #[test]
    fn a_file_without_recorded_content_becomes_a_todo() {
        let change = Change::Modified {
            before: entry("/etc/shadow", 0o000),
            after: entry("/etc/shadow", 0o000),
        };
        let mut runbook = step::build(&[], &[]);
        runbook.other_files.push(change);

        let playbook = rendered(&runbook, Path::new("/nonexistent"));
        assert!(playbook.yaml.contains("/etc/shadow"), "{}", playbook.yaml);
        assert!(
            !playbook.yaml.contains("ansible.builtin.copy"),
            "content is unknown, so there is nothing to copy: {}",
            playbook.yaml
        );
        assert_eq!(playbook.warnings.len(), 1);
    }

    /// The generator must carry a multibyte step title through untouched. The
    /// recording shell is a separate matter: bash 3.2 without a UTF-8 locale
    /// mangles such an argument before exarare ever sees it.
    #[test]
    fn keeps_a_multibyte_step_title() {
        let events = [
            crate::event::Event {
                ts: OffsetDateTime::UNIX_EPOCH,
                kind: crate::event::EventKind::Step {
                    title: "設定変更".into(),
                },
            },
            crate::event::Event {
                ts: OffsetDateTime::UNIX_EPOCH,
                kind: crate::event::EventKind::CmdStart {
                    cmd: "install -m 0644 /dev/null /opt/app/marker".into(),
                    cwd: "/root".into(),
                },
            },
            crate::event::Event {
                ts: OffsetDateTime::UNIX_EPOCH,
                kind: crate::event::EventKind::CmdEnd { exit_code: 0 },
            },
        ];
        let runbook = step::build(&events, &[]);
        let meta = meta();
        let playbook = render(
            &Input {
                meta: &meta,
                runbook: &runbook,
                blobs: Path::new("/nonexistent"),
                version: "0.1.0",
            },
            &Locale::Ja.messages(),
        );
        assert!(
            playbook.yaml.contains("# ステップ: 設定変更"),
            "{}",
            playbook.yaml
        );
        // Wrapping counts characters, not bytes, so a Japanese line is not cut
        // mid-character or padded to a shorter width.
        for line in playbook.yaml.lines() {
            assert!(line.chars().count() <= 160, "{line}");
        }
    }

    #[test]
    fn writes_the_playbook_and_its_files() {
        let tmp = tempfile::tempdir().unwrap();
        let playbook = Playbook {
            yaml: "---\n- name: test\n".into(),
            files: vec![(PathBuf::from("etc/app.conf"), "listen 80\n".into())],
            warnings: Vec::new(),
        };
        write(&playbook, tmp.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("playbook.yml")).unwrap(),
            "---\n- name: test\n"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("files/etc/app.conf")).unwrap(),
            "listen 80\n"
        );
    }
}
