//! Service, firewall, user and group state before and after a session.
//!
//! Like the package probe, this reads the system rather than the commands'
//! output, and records what it could not read instead of reporting nothing.
//! Persistent settings under `/etc` (sysctl drop-ins, unit overrides) are
//! already covered by the file snapshots; this is about state that lives
//! outside files.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::sys::{has_command, output};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
    pub shell: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub gid: u32,
    pub members: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Snapshot {
    /// Whether systemd could be asked at all.
    pub systemd: bool,
    pub enabled_units: Vec<String>,
    pub running_services: Vec<String>,
    /// Whether firewalld answered. False when it is absent or not running.
    pub firewalld: bool,
    pub firewall_services: Vec<String>,
    pub firewall_ports: Vec<String>,
    pub users: Vec<User>,
    pub groups: Vec<Group>,
    /// Commands or files that could not be read, with the reason.
    pub errors: Vec<String>,
}

/// First column of `systemctl list-unit-files` / `list-units`, which is the
/// unit name. A `●` marker on a failed unit is dropped.
pub fn parse_unit_names(text: &str) -> Vec<String> {
    let mut names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let line = line.trim_start_matches(['●', '*', ' ']);
            let name = line.split_whitespace().next()?;
            name.contains('.').then(|| name.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// `firewall-cmd --list-services` and `--list-ports` print one
/// whitespace-separated list.
pub fn parse_firewall_list(text: &str) -> Vec<String> {
    let mut items: Vec<String> = text.split_whitespace().map(str::to_string).collect();
    items.sort();
    items.dedup();
    items
}

/// `/etc/passwd`, which never holds a password hash.
pub fn parse_passwd(text: &str) -> Vec<User> {
    let mut users: Vec<User> = text
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            Some(User {
                name: f[0].to_string(),
                uid: f[2].parse().ok()?,
                gid: f[3].parse().ok()?,
                home: f[5].to_string(),
                shell: f[6].to_string(),
            })
        })
        .collect();
    users.sort_by(|a, b| a.name.cmp(&b.name));
    users
}

pub fn parse_group(text: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = text
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 4 {
                return None;
            }
            Some(Group {
                name: f[0].to_string(),
                gid: f[2].parse().ok()?,
                members: f[3]
                    .split(',')
                    .filter(|m| !m.is_empty())
                    .map(str::to_string)
                    .collect(),
            })
        })
        .collect();
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    groups
}

/// Reads the current state. Never fails as a whole: what could not be read is
/// reported in `errors`.
pub fn capture() -> Snapshot {
    let mut snapshot = Snapshot::default();

    if has_command("systemctl") {
        snapshot.systemd = true;
        match output(
            "systemctl",
            &[
                "list-unit-files",
                "--no-legend",
                "--no-pager",
                "--state=enabled",
            ],
        ) {
            Ok(text) => snapshot.enabled_units = parse_unit_names(&text),
            Err(e) => {
                snapshot.systemd = false;
                snapshot.errors.push(format!("{e:#}"));
            }
        }
        if snapshot.systemd {
            match output(
                "systemctl",
                &[
                    "list-units",
                    "--no-legend",
                    "--no-pager",
                    "--type=service",
                    "--state=running",
                ],
            ) {
                Ok(text) => snapshot.running_services = parse_unit_names(&text),
                Err(e) => snapshot.errors.push(format!("{e:#}")),
            }
        }
    }

    // firewalld only answers while its daemon runs, which is not the case in a
    // container. That is not an error worth shouting about, so it is recorded
    // as "not answered" rather than pushed into errors.
    if has_command("firewall-cmd") {
        let services = output("firewall-cmd", &["--permanent", "--list-services"]);
        let ports = output("firewall-cmd", &["--permanent", "--list-ports"]);
        if let (Ok(services), Ok(ports)) = (&services, &ports) {
            snapshot.firewalld = true;
            snapshot.firewall_services = parse_firewall_list(services);
            snapshot.firewall_ports = parse_firewall_list(ports);
        }
    }

    for (path, parse) in [("/etc/passwd", true), ("/etc/group", false)] {
        match std::fs::read_to_string(path) {
            Ok(text) if parse => snapshot.users = parse_passwd(&text),
            Ok(text) => snapshot.groups = parse_group(&text),
            Err(e) => snapshot.errors.push(format!("failed to read {path}: {e}")),
        }
    }
    snapshot
}

pub fn save(snapshot: &Snapshot, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(snapshot)? + "\n")
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Snapshot> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

#[derive(Debug, Clone, Default)]
pub struct Diff {
    pub systemd_known: bool,
    pub units_enabled: Vec<String>,
    pub units_disabled: Vec<String>,
    pub services_started: Vec<String>,
    pub services_stopped: Vec<String>,
    pub firewall_known: bool,
    pub firewall_services_added: Vec<String>,
    pub firewall_services_removed: Vec<String>,
    pub firewall_ports_added: Vec<String>,
    pub firewall_ports_removed: Vec<String>,
    pub users_added: Vec<User>,
    pub users_removed: Vec<User>,
    /// Same name, different uid, gid, home or shell.
    pub users_changed: Vec<(User, User)>,
    pub groups_added: Vec<Group>,
    pub groups_removed: Vec<Group>,
    pub errors: Vec<String>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.units_enabled.is_empty()
            && self.units_disabled.is_empty()
            && self.services_started.is_empty()
            && self.services_stopped.is_empty()
            && self.firewall_services_added.is_empty()
            && self.firewall_services_removed.is_empty()
            && self.firewall_ports_added.is_empty()
            && self.firewall_ports_removed.is_empty()
            && self.users_added.is_empty()
            && self.users_removed.is_empty()
            && self.users_changed.is_empty()
            && self.groups_added.is_empty()
            && self.groups_removed.is_empty()
    }

    /// Every name a step could be said to have touched, for attribution.
    pub fn names(&self) -> Vec<String> {
        let mut names = Vec::new();
        names.extend(self.units_enabled.iter().cloned());
        names.extend(self.services_started.iter().cloned());
        names.extend(self.users_added.iter().map(|u| u.name.clone()));
        names.extend(self.groups_added.iter().map(|g| g.name.clone()));
        names
    }
}

fn added(before: &[String], after: &[String]) -> Vec<String> {
    let before: BTreeSet<&String> = before.iter().collect();
    after
        .iter()
        .filter(|v| !before.contains(v))
        .cloned()
        .collect()
}

pub fn compare(before: &Snapshot, after: &Snapshot) -> Diff {
    let mut diff = Diff {
        systemd_known: before.systemd && after.systemd,
        firewall_known: before.firewalld && after.firewalld,
        ..Default::default()
    };
    diff.errors.extend(before.errors.iter().cloned());
    for error in &after.errors {
        if !diff.errors.contains(error) {
            diff.errors.push(error.clone());
        }
    }

    if diff.systemd_known {
        diff.units_enabled = added(&before.enabled_units, &after.enabled_units);
        diff.units_disabled = added(&after.enabled_units, &before.enabled_units);
        diff.services_started = added(&before.running_services, &after.running_services);
        diff.services_stopped = added(&after.running_services, &before.running_services);
    }
    if diff.firewall_known {
        diff.firewall_services_added = added(&before.firewall_services, &after.firewall_services);
        diff.firewall_services_removed = added(&after.firewall_services, &before.firewall_services);
        diff.firewall_ports_added = added(&before.firewall_ports, &after.firewall_ports);
        diff.firewall_ports_removed = added(&after.firewall_ports, &before.firewall_ports);
    }

    for user in &after.users {
        match before.users.iter().find(|u| u.name == user.name) {
            None => diff.users_added.push(user.clone()),
            Some(old) if old != user => diff.users_changed.push((old.clone(), user.clone())),
            Some(_) => {}
        }
    }
    for user in &before.users {
        if !after.users.iter().any(|u| u.name == user.name) {
            diff.users_removed.push(user.clone());
        }
    }
    for group in &after.groups {
        if !before.groups.iter().any(|g| g.name == group.name) {
            diff.groups_added.push(group.clone());
        }
    }
    for group in &before.groups {
        if !after.groups.iter().any(|g| g.name == group.name) {
            diff.groups_removed.push(group.clone());
        }
    }
    diff
}

/// A plain report for `exarare diff`, in English like the rest of the CLI.
pub fn render(diff: &Diff) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let list = |out: &mut String, label: &str, values: &[String]| {
        for value in values {
            let _ = writeln!(out, "{label:<12} {value}");
        }
    };
    list(&mut out, "enabled", &diff.units_enabled);
    list(&mut out, "disabled", &diff.units_disabled);
    list(&mut out, "started", &diff.services_started);
    list(&mut out, "stopped", &diff.services_stopped);
    list(&mut out, "fw add", &diff.firewall_services_added);
    list(&mut out, "fw remove", &diff.firewall_services_removed);
    list(&mut out, "port add", &diff.firewall_ports_added);
    list(&mut out, "port remove", &diff.firewall_ports_removed);
    for user in &diff.users_added {
        let _ = writeln!(
            out,
            "user added   {} (uid {}, shell {})",
            user.name, user.uid, user.shell
        );
    }
    for user in &diff.users_removed {
        let _ = writeln!(out, "user removed {}", user.name);
    }
    for (before, after) in &diff.users_changed {
        let _ = writeln!(
            out,
            "user changed {}: uid {} -> {}, shell {} -> {}",
            after.name, before.uid, after.uid, before.shell, after.shell
        );
    }
    for group in &diff.groups_added {
        let _ = writeln!(out, "group added  {} (gid {})", group.name, group.gid);
    }
    for group in &diff.groups_removed {
        let _ = writeln!(out, "group removed {}", group.name);
    }
    for error in &diff.errors {
        let _ = writeln!(out, "warning      {error}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unit_listings() {
        let text = "nginx.service                enabled enabled\n\
                    sshd.service                 enabled enabled\n\
                    \n";
        assert_eq!(
            parse_unit_names(text),
            vec!["nginx.service", "sshd.service"]
        );

        // `list-units` marks a failed unit with a bullet.
        let text = "● nginx.service loaded failed failed nginx\n\
                      sshd.service  loaded active running OpenSSH\n";
        assert_eq!(
            parse_unit_names(text),
            vec!["nginx.service", "sshd.service"]
        );
    }

    #[test]
    fn parses_firewall_lists() {
        assert_eq!(
            parse_firewall_list("dhcpv6-client http ssh\n"),
            vec!["dhcpv6-client", "http", "ssh"]
        );
        assert_eq!(parse_firewall_list("\n"), Vec::<String>::new());
    }

    #[test]
    fn parses_passwd_and_group() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\n\
                      # a comment\n\
                      deploy:x:1001:1001::/home/deploy:/bin/bash\n\
                      broken-line\n";
        let users = parse_passwd(passwd);
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].name, "deploy");
        assert_eq!(users[0].uid, 1001);
        assert_eq!(users[1].name, "root");

        let group = "wheel:x:10:deploy,ops\nempty:x:20:\n";
        let groups = parse_group(group);
        assert_eq!(groups[0].name, "empty");
        assert!(groups[0].members.is_empty());
        assert_eq!(groups[1].members, vec!["deploy", "ops"]);
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            systemd: true,
            firewalld: true,
            ..Default::default()
        }
    }

    #[test]
    fn notices_units_services_and_firewall_rules() {
        let mut before = snapshot();
        before.enabled_units = vec!["sshd.service".into()];
        before.running_services = vec!["sshd.service".into()];
        before.firewall_services = vec!["ssh".into()];
        let mut after = snapshot();
        after.enabled_units = vec!["nginx.service".into(), "sshd.service".into()];
        after.running_services = vec!["nginx.service".into()];
        after.firewall_services = vec!["http".into(), "ssh".into()];
        after.firewall_ports = vec!["8080/tcp".into()];

        let diff = compare(&before, &after);
        assert_eq!(diff.units_enabled, vec!["nginx.service"]);
        assert!(diff.units_disabled.is_empty());
        assert_eq!(diff.services_started, vec!["nginx.service"]);
        assert_eq!(diff.services_stopped, vec!["sshd.service"]);
        assert_eq!(diff.firewall_services_added, vec!["http"]);
        assert_eq!(diff.firewall_ports_added, vec!["8080/tcp"]);
    }

    #[test]
    fn notices_users_and_groups() {
        let user = |name: &str, uid: u32, shell: &str| User {
            name: name.into(),
            uid,
            gid: uid,
            home: format!("/home/{name}"),
            shell: shell.into(),
        };
        let mut before = snapshot();
        before.users = vec![user("root", 0, "/bin/bash"), user("old", 1001, "/bin/bash")];
        before.groups = vec![Group {
            name: "wheel".into(),
            gid: 10,
            members: vec![],
        }];
        let mut after = snapshot();
        after.users = vec![
            user("deploy", 1002, "/bin/bash"),
            user("root", 0, "/bin/sh"),
        ];
        after.groups = vec![
            Group {
                name: "deploy".into(),
                gid: 1002,
                members: vec!["deploy".into()],
            },
            Group {
                name: "wheel".into(),
                gid: 10,
                members: vec![],
            },
        ];

        let diff = compare(&before, &after);
        assert_eq!(
            diff.users_added.iter().map(|u| &u.name).collect::<Vec<_>>(),
            vec!["deploy"]
        );
        assert_eq!(
            diff.users_removed
                .iter()
                .map(|u| &u.name)
                .collect::<Vec<_>>(),
            vec!["old"]
        );
        assert_eq!(diff.users_changed.len(), 1, "root's shell changed");
        assert_eq!(diff.users_changed[0].1.shell, "/bin/sh");
        assert_eq!(
            diff.groups_added
                .iter()
                .map(|g| &g.name)
                .collect::<Vec<_>>(),
            vec!["deploy"]
        );
    }

    /// A container has no running systemd or firewalld; that must read as
    /// "unknown", not as everything having been switched off.
    #[test]
    fn unavailable_subsystems_report_nothing() {
        let before = Snapshot {
            enabled_units: vec!["sshd.service".into()],
            firewall_services: vec!["ssh".into()],
            ..Default::default()
        };
        let after = Snapshot::default();

        let diff = compare(&before, &after);
        assert!(!diff.systemd_known);
        assert!(!diff.firewall_known);
        assert!(diff.is_empty(), "{diff:?}");
    }

    #[test]
    fn collects_names_for_attribution() {
        let mut before = snapshot();
        before.enabled_units = vec![];
        let mut after = snapshot();
        after.enabled_units = vec!["nginx.service".into()];
        after.running_services = vec!["nginx.service".into()];
        after.users = vec![User {
            name: "deploy".into(),
            uid: 1001,
            gid: 1001,
            home: "/home/deploy".into(),
            shell: "/bin/bash".into(),
        }];

        let diff = compare(&before, &after);
        assert!(diff.names().contains(&"nginx.service".to_string()));
        assert!(diff.names().contains(&"deploy".to_string()));
    }
}
