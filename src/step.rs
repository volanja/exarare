//! Turns recorded events and file changes into the steps a runbook is made of.

use std::path::Path;

use time::OffsetDateTime;

use crate::diff::Change;
use crate::event::{Event, EventKind};

/// One command as it ran.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandRun {
    pub ts: OffsetDateTime,
    pub cmd: String,
    pub cwd: String,
    pub exit_code: Option<i32>,
}

impl CommandRun {
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// The body of a step, in the order it was recorded, so that a remark stays
/// next to the command it is about.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Command(CommandRun),
    Note(String),
}

/// A section of the runbook: what happened under one `exarare step` heading.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// `None` for work done before the first step, or when none was recorded.
    pub title: Option<String>,
    pub items: Vec<Item>,
    /// File changes a command of this step names by path.
    pub files: Vec<Change>,
}

impl Step {
    pub fn commands(&self) -> impl Iterator<Item = &CommandRun> {
        self.items.iter().filter_map(|item| match item {
            Item::Command(run) => Some(run),
            Item::Note(_) => None,
        })
    }

    pub fn command_count(&self) -> usize {
        self.commands().count()
    }
}

#[derive(Debug, Clone)]
pub struct Runbook {
    pub steps: Vec<Step>,
    /// Everything that ran, in order, for the appendix.
    pub log: Vec<CommandRun>,
    /// Changes no step claimed.
    pub other_files: Vec<Change>,
    /// Successful commands that look like checks.
    pub verifications: Vec<CommandRun>,
    /// True when the operator never ran `exarare step`.
    pub without_steps: bool,
}

/// Commands that inspect rather than change, so they are not instructions.
const NOISE: &[&str] = &[
    "exit", "logout", "clear", "pwd", "history", "ls", "ll", "la", "cat", "less", "more", "tail",
    "head", "man", "top", "htop", "df", "free", "id", "whoami", "date", "env", "printenv", "which",
    "type", "ps", "grep", "find", "diff", "vi", "vim", "nano", "emacs", "view",
];

/// Commands that read state back, and so belong in the verification chapter.
const VERIFICATION: &[&str] = &[
    "systemctl status",
    "systemctl is-active",
    "systemctl is-enabled",
    "systemctl list-unit-files",
    "journalctl",
    "curl",
    "wget -q -O-",
    "ss ",
    "netstat",
    "ping",
    "dig",
    "nslookup",
    "rpm -q",
    "dnf list installed",
    "firewall-cmd --list",
    "getenforce",
    "sestatus",
    "nginx -t",
    "httpd -t",
    "sshd -t",
    "openssl s_client",
];

/// First word of the command, with `sudo` and leading environment assignments
/// stripped, so `sudo dnf install` is judged on `dnf`.
fn head(cmd: &str) -> &str {
    let mut rest = cmd.trim();
    loop {
        let word = rest.split_whitespace().next().unwrap_or("");
        let is_env_assignment = word.contains('=') && !word.starts_with('=');
        if word == "sudo" || word == "command" || is_env_assignment {
            rest = rest[word.len()..].trim_start();
            continue;
        }
        return word;
    }
}

fn is_noise(cmd: &str) -> bool {
    let head = head(cmd);
    // exarare's own subcommands are bookkeeping, not part of the work.
    head == "exarare" || NOISE.contains(&head)
}

fn is_verification(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    VERIFICATION.iter().any(|p| trimmed.contains(p))
        || trimmed.ends_with("--version")
        || trimmed.ends_with("-V")
}

/// Whether a command line qualifies as an instruction of the procedure.
fn is_instruction(run: &CommandRun) -> bool {
    run.succeeded() && !is_noise(&run.cmd) && !is_verification(&run.cmd)
}

/// Paths named in the command line, matched against the changed files.
fn mentions(cmd: &str, path: &Path) -> bool {
    if cmd.contains(path.to_string_lossy().as_ref()) {
        return true;
    }
    // Fall back to the last component of each word. A snapshot path is
    // canonical (`/private/var/...` on macOS, `/etc` resolved through symlinks)
    // while the command holds whatever the operator typed, so the full strings
    // often differ even for the same file. This also catches `vi nginx.conf`
    // run inside the directory.
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.len() <= 3 {
        return false;
    }
    cmd.split_whitespace().any(|word| {
        let word = word.trim_matches(|c| c == '"' || c == '\'' || c == '>' || c == '<');
        word.rsplit('/').next() == Some(name)
    })
}

/// A recorded item before the exit codes are known: commands are held as
/// indices into the log, which is filled in as `CmdEnd` events arrive.
enum Recorded {
    Command(usize),
    Note(String),
}

/// Builds the runbook model. `changes` is session-wide, because snapshots are
/// taken once before and once after.
pub fn build(events: &[Event], changes: &[Change]) -> Runbook {
    let mut log: Vec<CommandRun> = Vec::new();
    let mut groups: Vec<(Option<String>, Vec<Recorded>)> = vec![(None, Vec::new())];

    for event in events {
        let group = groups.last_mut().expect("one group always exists");
        match &event.kind {
            EventKind::CmdStart { cmd, cwd } => {
                log.push(CommandRun {
                    ts: event.ts,
                    cmd: cmd.clone(),
                    cwd: cwd.clone(),
                    exit_code: None,
                });
                group.1.push(Recorded::Command(log.len() - 1));
            }
            EventKind::CmdEnd { exit_code } => {
                if let Some(run) = log.last_mut() {
                    run.exit_code = Some(*exit_code);
                }
            }
            EventKind::Note { text } => group.1.push(Recorded::Note(text.clone())),
            EventKind::Step { title } => groups.push((Some(title.clone()), Vec::new())),
            EventKind::SessionStart | EventKind::SessionEnd => {}
        }
    }

    let without_steps = groups.iter().all(|(title, _)| title.is_none());

    let verifications: Vec<CommandRun> = log
        .iter()
        .filter(|run| run.succeeded() && is_verification(&run.cmd))
        .cloned()
        .collect();

    // Instructions leave out failures, inspection and checks. Checks are shown
    // in their own chapter, so they would otherwise appear twice. Notes keep
    // their place among the commands they were written about.
    let mut steps: Vec<(Step, Vec<&CommandRun>)> = groups
        .iter()
        .map(|(title, recorded)| {
            let items = recorded
                .iter()
                .filter_map(|item| match item {
                    Recorded::Command(i) => {
                        let run = &log[*i];
                        is_instruction(run).then(|| Item::Command(run.clone()))
                    }
                    Recorded::Note(text) => Some(Item::Note(text.clone())),
                })
                .collect();
            let all_commands = recorded
                .iter()
                .filter_map(|item| match item {
                    Recorded::Command(i) => Some(&log[*i]),
                    Recorded::Note(_) => None,
                })
                .collect();
            (
                Step {
                    title: title.clone(),
                    items,
                    files: Vec::new(),
                },
                all_commands,
            )
        })
        .filter(|(step, _)| !step.items.is_empty() || step.title.is_some())
        .collect();

    // Attribution looks at every command of the step, not just the
    // instructions: `vi /etc/nginx/nginx.conf` is dropped as inspection, yet it
    // is the line that names the file that changed.
    let mut other_files = Vec::new();
    for change in changes {
        let owner = steps
            .iter_mut()
            .find(|(_, runs)| runs.iter().any(|run| mentions(&run.cmd, change.path())));
        match owner {
            Some((step, _)) => step.files.push(change.clone()),
            None => other_files.push(change.clone()),
        }
    }
    let steps: Vec<Step> = steps.into_iter().map(|(step, _)| step).collect();

    Runbook {
        steps,
        log,
        other_files,
        verifications,
        without_steps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{Content, Entry};
    use std::path::PathBuf;

    fn event(kind: EventKind) -> Event {
        Event {
            ts: OffsetDateTime::UNIX_EPOCH,
            kind,
        }
    }

    fn cmd(cmd: &str) -> Event {
        event(EventKind::CmdStart {
            cmd: cmd.into(),
            cwd: "/root".into(),
        })
    }

    fn done(exit_code: i32) -> Event {
        event(EventKind::CmdEnd { exit_code })
    }

    fn step(title: &str) -> Event {
        event(EventKind::Step {
            title: title.into(),
        })
    }

    fn note(text: &str) -> Event {
        event(EventKind::Note { text: text.into() })
    }

    fn entry(path: &str) -> Entry {
        Entry {
            path: PathBuf::from(path),
            content: Content::File {
                size: 1,
                hash: "h".into(),
                blob: false,
                masked: false,
            },
            mode: 0o644,
            uid: 0,
            gid: 0,
        }
    }

    fn command_lines(step: &Step) -> Vec<&str> {
        step.commands().map(|run| run.cmd.as_str()).collect()
    }

    #[test]
    fn splits_on_steps_and_drops_failures_and_noise() {
        let events = vec![
            event(EventKind::SessionStart),
            step("Install nginx"),
            cmd("dnf install -y nginx"),
            done(0),
            cmd("ls /etc/nginx"),
            done(0),
            cmd("systemctl start nginx-typo"),
            done(1),
            cmd("systemctl start nginx"),
            done(0),
            step("Open the firewall"),
            cmd("firewall-cmd --add-service=http --permanent"),
            done(0),
            cmd("exit"),
        ];

        let runbook = build(&events, &[]);
        assert!(!runbook.without_steps);
        let steps: Vec<(&str, Vec<&str>)> = runbook
            .steps
            .iter()
            .map(|s| (s.title.as_deref().unwrap_or(""), command_lines(s)))
            .collect();
        assert_eq!(
            steps,
            vec![
                (
                    "Install nginx",
                    vec!["dnf install -y nginx", "systemctl start nginx"]
                ),
                (
                    "Open the firewall",
                    vec!["firewall-cmd --add-service=http --permanent"]
                ),
            ]
        );
        // Nothing is lost: the appendix keeps the failure and the noise.
        assert_eq!(runbook.log.len(), 6);
        assert!(runbook.log.iter().any(|r| r.exit_code == Some(1)));
    }

    #[test]
    fn a_note_stays_between_the_commands_it_was_written_about() {
        let events = vec![
            step("Install nginx"),
            cmd("dnf install -y nginx"),
            done(0),
            note("needs EPEL enabled"),
            cmd("systemctl enable --now nginx"),
            done(0),
        ];

        let runbook = build(&events, &[]);
        let items = &runbook.steps[0].items;
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[0], Item::Command(run) if run.cmd == "dnf install -y nginx"));
        assert_eq!(items[1], Item::Note("needs EPEL enabled".into()));
        assert!(matches!(&items[2], Item::Command(run) if run.cmd.starts_with("systemctl enable")));
        // A note is not a step boundary.
        assert_eq!(runbook.steps.len(), 1);
    }

    #[test]
    fn a_session_without_steps_is_one_step() {
        let events = vec![cmd("dnf install -y zsh"), done(0)];
        let runbook = build(&events, &[]);
        assert!(runbook.without_steps);
        assert_eq!(runbook.steps.len(), 1);
        assert_eq!(runbook.steps[0].title, None);
    }

    #[test]
    fn attributes_files_to_the_step_that_names_them() {
        let events = vec![
            step("Configure nginx"),
            cmd("vi /etc/nginx/nginx.conf"),
            done(0),
            step("Tune the kernel"),
            cmd("sysctl -p"),
            done(0),
        ];
        let changes = vec![
            Change::Modified {
                before: entry("/etc/nginx/nginx.conf"),
                after: entry("/etc/nginx/nginx.conf"),
            },
            Change::Added(entry("/etc/sysctl.d/99-tuning.conf")),
        ];

        let runbook = build(&events, &changes);
        assert_eq!(runbook.steps[0].files.len(), 1);
        assert_eq!(
            runbook.steps[0].files[0].path(),
            Path::new("/etc/nginx/nginx.conf")
        );
        // `sysctl -p` does not name the file, so it stays unattributed.
        assert_eq!(runbook.steps[1].files.len(), 0);
        assert_eq!(runbook.other_files.len(), 1);
    }

    #[test]
    fn attributes_files_across_a_symlinked_prefix() {
        // The snapshot path is canonical, the command is not: on macOS a
        // temporary directory is recorded under /private/var but typed as /var.
        let events = vec![
            step("Write the config"),
            cmd("printf 'listen 443\\n' > /var/folders/x/T/tmp1/etc/app.conf"),
            done(0),
        ];
        let changes = vec![Change::Added(entry(
            "/private/var/folders/x/T/tmp1/etc/app.conf",
        ))];

        let runbook = build(&events, &changes);
        assert_eq!(runbook.steps[0].files.len(), 1);
        assert!(runbook.other_files.is_empty());
    }

    #[test]
    fn collects_verification_commands() {
        let events = vec![
            cmd("systemctl status nginx"),
            done(0),
            cmd("curl -sS localhost"),
            done(0),
            cmd("nginx -t"),
            done(1),
            cmd("dnf install -y nginx"),
            done(0),
        ];
        let runbook = build(&events, &[]);
        let found: Vec<&str> = runbook
            .verifications
            .iter()
            .map(|r| r.cmd.as_str())
            .collect();
        // The failed check is not offered as a verification step.
        assert_eq!(found, vec!["systemctl status nginx", "curl -sS localhost"]);
    }

    #[test]
    fn looks_past_sudo_and_environment_assignments() {
        assert_eq!(head("sudo dnf install -y nginx"), "dnf");
        assert_eq!(head("LANG=C sudo rpm -qa"), "rpm");
        assert!(is_noise("sudo cat /etc/hosts"));
        assert!(!is_noise("sudo dnf install -y nginx"));
    }
}
