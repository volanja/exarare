//! Package state before and after a session.
//!
//! What a `dnf install` actually did is read from the rpm database rather than
//! from the command's output, which differs between dnf 4 and dnf 5 and says
//! nothing when the install came from a script. Commands that are missing or
//! fail are recorded, so a report can say what could not be read instead of
//! quietly claiming nothing changed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::sys::{has_command, output};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Package {
    pub name: String,
    /// Epoch, version and release, as rpm prints `%{EVR}`.
    pub version: String,
    pub arch: String,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Snapshot {
    /// False when the host has no rpm, so nothing could be read.
    pub available: bool,
    pub packages: Vec<Package>,
    /// Names dnf reports as installed on purpose, rather than as dependencies.
    /// Empty when dnf could not answer; see `errors`.
    pub user_installed: Vec<String>,
    pub repositories: Vec<String>,
    /// Enabled module streams, as dnf prints them (EL8 and EL9).
    pub modules: Vec<String>,
    /// Commands that could not be run, with the reason.
    pub errors: Vec<String>,
}

impl Snapshot {
    pub fn knows_user_installed(&self) -> bool {
        !self.user_installed.is_empty()
    }
}

/// `name\tEVR\tarch` per line, as produced by `rpm -qa --qf`.
pub fn parse_rpm_qa(text: &str) -> Vec<Package> {
    let mut packages: Vec<Package> = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let name = fields.next()?.trim();
            let version = fields.next()?.trim();
            let arch = fields.next()?.trim();
            if name.is_empty() {
                return None;
            }
            Some(Package {
                name: name.to_string(),
                version: version.to_string(),
                arch: arch.to_string(),
            })
        })
        .collect();
    packages.sort();
    packages
}

/// One name per line, ignoring dnf's warnings and blank lines.
pub fn parse_names(text: &str) -> Vec<String> {
    let mut names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.contains(' ')
                && !line.starts_with('#')
                && !line.ends_with(':')
        })
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// First column of `dnf repolist --enabled`, without its header.
pub fn parse_repolist(text: &str) -> Vec<String> {
    let mut repos: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            !lower.starts_with("repo id") && !lower.starts_with("last metadata")
        })
        .filter_map(|line| line.split_whitespace().next().map(str::to_string))
        .collect();
    repos.sort();
    repos.dedup();
    repos
}

/// Lines of `dnf module list --enabled` that name a module, kept verbatim
/// because the columns differ between releases.
pub fn parse_modules(text: &str) -> Vec<String> {
    let mut modules: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            !lower.starts_with("name ")
                && !lower.starts_with("last metadata")
                && !lower.contains("hint:")
                && !lower.ends_with("repository")
        })
        .map(str::to_string)
        .collect();
    modules.sort();
    modules.dedup();
    modules
}

/// Reads the current package state. Never fails as a whole: what could not be
/// read is reported in `errors`.
pub fn capture() -> Snapshot {
    let mut snapshot = Snapshot::default();
    if !has_command("rpm") {
        return snapshot;
    }
    snapshot.available = true;

    match output("rpm", &["-qa", "--qf", "%{NAME}\t%{EVR}\t%{ARCH}\n"]) {
        Ok(text) => snapshot.packages = parse_rpm_qa(&text),
        Err(e) => snapshot.errors.push(format!("{e:#}")),
    }

    if !has_command("dnf") {
        snapshot
            .errors
            .push("dnf is not installed: packages cannot be told apart from their dependencies, and repositories are unknown".into());
        return snapshot;
    }

    // -C keeps dnf off the network: a snapshot must not wait on repository
    // metadata, and everything read here comes from the local state anyway.
    match output(
        "dnf",
        &[
            "-q",
            "-C",
            "repoquery",
            "--userinstalled",
            "--queryformat",
            "%{name}",
        ],
    ) {
        Ok(text) => snapshot.user_installed = parse_names(&text),
        Err(e) => snapshot.errors.push(format!("{e:#}")),
    }
    match output("dnf", &["-q", "-C", "repolist", "--enabled"]) {
        Ok(text) => snapshot.repositories = parse_repolist(&text),
        Err(e) => snapshot.errors.push(format!("{e:#}")),
    }
    // Modules exist on EL8 and EL9 only; a failure here is expected elsewhere.
    if let Ok(text) = output("dnf", &["-q", "-C", "module", "list", "--enabled"]) {
        snapshot.modules = parse_modules(&text);
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upgrade {
    pub name: String,
    pub arch: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default)]
pub struct Diff {
    /// False when either snapshot could not be taken, so nothing is known.
    pub available: bool,
    pub installed: Vec<Package>,
    pub removed: Vec<Package>,
    pub upgraded: Vec<Upgrade>,
    /// Of the installed packages, those dnf says were asked for by name.
    pub explicit: BTreeSet<String>,
    /// Whether dnf could tell explicit installs from dependencies at all.
    pub explicit_known: bool,
    pub repositories_added: Vec<String>,
    pub repositories_removed: Vec<String>,
    pub modules_added: Vec<String>,
    /// Reasons the picture may be incomplete, from either snapshot.
    pub errors: Vec<String>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.installed.is_empty()
            && self.removed.is_empty()
            && self.upgraded.is_empty()
            && self.repositories_added.is_empty()
            && self.repositories_removed.is_empty()
            && self.modules_added.is_empty()
    }

    /// Installed packages the operator asked for by name. With no answer from
    /// dnf, every installed package is treated as explicit: hiding them would
    /// leave the runbook with nothing to say.
    pub fn explicit_installs(&self) -> Vec<&Package> {
        self.installed
            .iter()
            .filter(|p| !self.explicit_known || self.explicit.contains(&p.name))
            .collect()
    }

    /// Installed only to satisfy something else.
    pub fn dependency_installs(&self) -> Vec<&Package> {
        self.installed
            .iter()
            .filter(|p| self.explicit_known && !self.explicit.contains(&p.name))
            .collect()
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
        available: before.available && after.available,
        explicit_known: after.knows_user_installed(),
        ..Default::default()
    };
    diff.errors.extend(before.errors.iter().cloned());
    for error in &after.errors {
        if !diff.errors.contains(error) {
            diff.errors.push(error.clone());
        }
    }
    if !diff.available {
        return diff;
    }

    let index = |packages: &[Package]| -> BTreeMap<(String, String), String> {
        packages
            .iter()
            .map(|p| ((p.name.clone(), p.arch.clone()), p.version.clone()))
            .collect()
    };
    let before_index = index(&before.packages);

    for package in &after.packages {
        let key = (package.name.clone(), package.arch.clone());
        match before_index.get(&key) {
            None => diff.installed.push(package.clone()),
            Some(version) if version != &package.version => diff.upgraded.push(Upgrade {
                name: package.name.clone(),
                arch: package.arch.clone(),
                from: version.clone(),
                to: package.version.clone(),
            }),
            Some(_) => {}
        }
    }
    let after_index = index(&after.packages);
    for package in &before.packages {
        if !after_index.contains_key(&(package.name.clone(), package.arch.clone())) {
            diff.removed.push(package.clone());
        }
    }

    diff.explicit = added(&before.user_installed, &after.user_installed)
        .into_iter()
        .collect();
    diff.repositories_added = added(&before.repositories, &after.repositories);
    diff.repositories_removed = added(&after.repositories, &before.repositories);
    diff.modules_added = added(&before.modules, &after.modules);
    diff
}

/// A plain report for `exarare diff`, in English like the rest of the CLI.
pub fn render(diff: &Diff) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for package in diff.explicit_installs() {
        let _ = writeln!(
            out,
            "installed    {}-{}.{}",
            package.name, package.version, package.arch
        );
    }
    let dependencies = diff.dependency_installs();
    if !dependencies.is_empty() {
        let _ = writeln!(
            out,
            "installed    {} package(s) as dependencies",
            dependencies.len()
        );
    }
    for package in &diff.removed {
        let _ = writeln!(
            out,
            "removed      {}-{}.{}",
            package.name, package.version, package.arch
        );
    }
    for upgrade in &diff.upgraded {
        let _ = writeln!(
            out,
            "upgraded     {} {} -> {}",
            upgrade.name, upgrade.from, upgrade.to
        );
    }
    for repo in &diff.repositories_added {
        let _ = writeln!(out, "repo added   {repo}");
    }
    for repo in &diff.repositories_removed {
        let _ = writeln!(out, "repo removed {repo}");
    }
    for module in &diff.modules_added {
        let _ = writeln!(out, "module       {module}");
    }
    for error in &diff.errors {
        let _ = writeln!(out, "warning      {error}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, version: &str) -> Package {
        Package {
            name: name.into(),
            version: version.into(),
            arch: "x86_64".into(),
        }
    }

    fn snapshot(packages: &[Package], user_installed: &[&str]) -> Snapshot {
        Snapshot {
            available: true,
            packages: packages.to_vec(),
            user_installed: user_installed.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn parses_rpm_output() {
        let text = "nginx\t1:1.20.1-14.el9_2.1\tx86_64\n\
                    nginx-filesystem\t1:1.20.1-14.el9_2.1\tnoarch\n\
                    \n";
        let packages = parse_rpm_qa(text);
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "nginx");
        assert_eq!(packages[0].version, "1:1.20.1-14.el9_2.1");
        assert_eq!(packages[1].arch, "noarch");
    }

    #[test]
    fn parses_names_and_ignores_noise() {
        let text = "Last metadata expiration check: 0:00:01 ago\nnginx\nzsh\nzsh\n\n";
        assert_eq!(parse_names(text), vec!["nginx", "zsh"]);
    }

    #[test]
    fn parses_repolist_without_its_header() {
        let text = "repo id                 repo name\n\
                    appstream               AlmaLinux 9 - AppStream\n\
                    baseos                  AlmaLinux 9 - BaseOS\n";
        assert_eq!(parse_repolist(text), vec!["appstream", "baseos"]);
    }

    #[test]
    fn classifies_package_changes() {
        let before = snapshot(
            &[package("bash", "5.1.8-6.el9"), package("sed", "4.8-9.el9")],
            &["bash"],
        );
        let after = snapshot(
            &[
                package("bash", "5.1.8-9.el9"),
                package("nginx", "1:1.20.1-14.el9"),
                package("nginx-core", "1:1.20.1-14.el9"),
            ],
            &["bash", "nginx"],
        );

        let diff = compare(&before, &after);
        assert_eq!(
            diff.installed.iter().map(|p| &p.name).collect::<Vec<_>>(),
            vec!["nginx", "nginx-core"]
        );
        assert_eq!(
            diff.removed.iter().map(|p| &p.name).collect::<Vec<_>>(),
            vec!["sed"]
        );
        assert_eq!(
            diff.upgraded,
            vec![Upgrade {
                name: "bash".into(),
                arch: "x86_64".into(),
                from: "5.1.8-6.el9".into(),
                to: "5.1.8-9.el9".into(),
            }]
        );
        // nginx was asked for, nginx-core came along with it.
        assert!(diff.explicit_known);
        assert_eq!(
            diff.explicit_installs()
                .iter()
                .map(|p| &p.name)
                .collect::<Vec<_>>(),
            vec!["nginx"]
        );
        assert_eq!(
            diff.dependency_installs()
                .iter()
                .map(|p| &p.name)
                .collect::<Vec<_>>(),
            vec!["nginx-core"]
        );
    }

    #[test]
    fn without_an_answer_from_dnf_every_install_is_explicit() {
        let before = snapshot(&[], &[]);
        let after = snapshot(&[package("nginx", "1:1.20.1-14.el9")], &[]);
        let diff = compare(&before, &after);
        assert!(!diff.explicit_known);
        assert_eq!(diff.explicit_installs().len(), 1);
        assert!(diff.dependency_installs().is_empty());
    }

    #[test]
    fn an_unavailable_snapshot_reports_nothing_rather_than_removals() {
        let before = snapshot(&[package("bash", "5.1.8-6.el9")], &["bash"]);
        let after = Snapshot::default();
        let diff = compare(&before, &after);
        assert!(
            diff.is_empty(),
            "a host without rpm must not look like a wipe"
        );
    }

    #[test]
    fn carries_errors_from_both_snapshots() {
        let mut before = snapshot(&[], &[]);
        before.errors.push("rpm failed".into());
        let mut after = snapshot(&[], &[]);
        after.errors.push("dnf failed".into());
        let diff = compare(&before, &after);
        assert_eq!(diff.errors, vec!["rpm failed", "dnf failed"]);
    }

    #[test]
    fn notices_new_repositories_and_modules() {
        let mut before = snapshot(&[], &[]);
        before.repositories = vec!["baseos".into()];
        let mut after = snapshot(&[], &[]);
        after.repositories = vec!["baseos".into(), "epel".into()];
        after.modules = vec!["nodejs 20 [e]".into()];
        let diff = compare(&before, &after);
        assert_eq!(diff.repositories_added, vec!["epel"]);
        assert!(diff.repositories_removed.is_empty());
        assert_eq!(diff.modules_added, vec!["nodejs 20 [e]"]);
    }
}
