//! Watches directories for files that were touched during a session.
//!
//! This complements the snapshots rather than replacing them. A snapshot says
//! what a file's content became; the watcher says a path was written to at all,
//! which covers two cases the snapshots miss: a directory nobody thought to
//! snapshot, and a file that was edited and then put back.
//!
//! Only paths are recorded, never content. The set is deduplicated and capped,
//! and when a limit is reached that fact is recorded rather than hidden.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use notify::{EventKind, RecursiveMode, Watcher as _};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Directories that are watched for touches, when they exist. They are not
/// snapshotted, so nothing is diffed there: the point is to notice work in a
/// place the operator forgot to pass to `--snapshot`.
pub fn default_roots() -> Vec<PathBuf> {
    ["/opt", "/usr/local", "/srv", "/var/www", "/root", "/home"]
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .collect()
}

/// Paths that say nothing about the work, or that would never stop reporting.
const EXCLUDE: &[&str] = &[
    // Kernel and runtime interfaces rather than files.
    "/proc/**",
    "/sys/**",
    "/dev/**",
    "/run/**",
    "/var/run/**",
    // Scratch space.
    "/tmp/**",
    "/var/tmp/**",
    // Logs, which are written continuously and say nothing about the change.
    "/var/log/**",
    // Package manager internals; package changes come from the rpm probe.
    "/var/lib/rpm/**",
    "/var/lib/dnf/**",
    "/var/cache/**",
    // Runtime state; service changes come from the systemd probe.
    "/var/lib/systemd/**",
    "/var/lib/selinux/**",
    "/var/spool/**",
    // Container storage, which moves in whole layers.
    "/var/lib/containers/**",
    "/var/lib/docker/**",
    // Shell history is not part of the work.
    "**/.bash_history",
    "**/.zsh_history",
    "**/.sh_history",
    // Editors and their temporary files.
    "**/*.swp",
    "**/*.swx",
    "**/*~",
    "**/4913",
    "**/.#*",
    "**/#*#",
    // Implementation leftovers.
    "**/*.lock",
    "**/__pycache__/**",
    "**/.git/**",
];

pub struct Rules {
    exclude: GlobSet,
    /// exarare's own data directory: the session writes to it constantly, so
    /// watching it would report exarare watching itself.
    own_data: Option<PathBuf>,
}

impl Rules {
    pub fn new(own_data: Option<PathBuf>) -> Result<Self> {
        let mut builder = GlobSetBuilder::new();
        for pattern in EXCLUDE {
            builder.add(Glob::new(pattern).with_context(|| format!("bad pattern {pattern}"))?);
        }
        Ok(Self {
            exclude: builder.build()?,
            own_data: own_data.and_then(|p| p.canonicalize().ok().or(Some(p))),
        })
    }

    pub fn is_excluded(&self, path: &Path) -> bool {
        if self.exclude.is_match(path) {
            return true;
        }
        self.own_data
            .as_ref()
            .is_some_and(|own| path.starts_with(own))
    }
}

/// At most this many paths are remembered. A loop that writes forever should
/// not be able to grow the session without bound.
pub const MAX_PATHS: usize = 10_000;

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Touched {
    /// Path to the time it was first seen, which is what places it in a step.
    #[serde(with = "paths_as_pairs")]
    pub paths: BTreeMap<PathBuf, OffsetDateTime>,
    /// More paths were touched than `MAX_PATHS`, so the list is incomplete.
    pub truncated: bool,
    /// Watching a directory failed, usually because the kernel's watch limit
    /// was reached, so touches under it were missed.
    pub incomplete_roots: Vec<String>,
    /// Roots that were watched, for the report to say what was covered.
    pub roots: Vec<PathBuf>,
}

/// `BTreeMap<PathBuf, OffsetDateTime>` has no serde support for the time type
/// as a map value, so it is stored as a list of pairs.
mod paths_as_pairs {
    use super::*;
    use serde::{Deserializer, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Pair {
        path: PathBuf,
        #[serde(with = "time::serde::rfc3339")]
        first_seen: OffsetDateTime,
    }

    pub fn serialize<S: Serializer>(
        value: &BTreeMap<PathBuf, OffsetDateTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let pairs: Vec<Pair> = value
            .iter()
            .map(|(path, first_seen)| Pair {
                path: path.clone(),
                first_seen: *first_seen,
            })
            .collect();
        serde::Serialize::serialize(&pairs, serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<PathBuf, OffsetDateTime>, D::Error> {
        let pairs: Vec<Pair> = serde::Deserialize::deserialize(deserializer)?;
        Ok(pairs
            .into_iter()
            .map(|pair| (pair.path, pair.first_seen))
            .collect())
    }
}

impl Touched {
    /// Paths first seen in `[from, until)`, or from `from` onwards when `until`
    /// is `None`. A step's window runs until the next step begins rather than
    /// until its own last command: a watcher reports with some delay — FSEvents
    /// noticeably so — and a write during the work belongs to the step that was
    /// running, however late the notification arrives.
    pub fn in_window(&self, from: OffsetDateTime, until: Option<OffsetDateTime>) -> Vec<&PathBuf> {
        self.paths
            .iter()
            .filter(|(_, seen)| **seen >= from && until.is_none_or(|until| **seen < until))
            .map(|(path, _)| path)
            .collect()
    }
}

/// Records a touched path, respecting the cap.
fn record(touched: &mut Touched, path: PathBuf, at: OffsetDateTime) {
    if touched.paths.contains_key(&path) {
        return;
    }
    if touched.paths.len() >= MAX_PATHS {
        touched.truncated = true;
        return;
    }
    touched.paths.insert(path, at);
}

/// A running watcher. Dropping it stops watching; call `finish` to read what
/// was seen.
pub struct Watcher {
    inner: Option<notify::RecommendedWatcher>,
    touched: Arc<Mutex<Touched>>,
}

pub fn start(roots: &[PathBuf], rules: Rules) -> Result<Watcher> {
    let touched = Arc::new(Mutex::new(Touched {
        roots: roots.to_vec(),
        ..Default::default()
    }));
    let shared = Arc::clone(&touched);

    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        // Reads are not work. Metadata changes are, since a chmod is a change.
        if !matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        ) {
            return;
        }
        let now = OffsetDateTime::now_utc();
        let Ok(mut touched) = shared.lock() else {
            return;
        };
        for path in event.paths {
            if rules.is_excluded(&path) {
                continue;
            }
            record(&mut touched, path, now);
        }
    })
    .context("failed to start the filesystem watcher")?;

    let mut incomplete = Vec::new();
    for root in roots {
        if let Err(e) = watcher.watch(root, RecursiveMode::Recursive) {
            // A missing directory or an exhausted watch limit must not stop the
            // session; it is recorded so the report can admit the gap.
            incomplete.push(format!("{}: {e}", root.display()));
        }
    }
    if let Ok(mut guard) = touched.lock() {
        guard.incomplete_roots = incomplete;
    }

    Ok(Watcher {
        inner: Some(watcher),
        touched,
    })
}

impl Watcher {
    /// Stops watching and returns what was seen.
    pub fn finish(mut self) -> Touched {
        // Dropping the notify watcher stops its thread, so nothing can be added
        // while the result is read.
        self.inner.take();
        self.touched
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

pub fn save(touched: &Touched, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(touched)? + "\n")
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Touched> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Rules {
        Rules::new(Some(PathBuf::from("/var/lib/exarare"))).unwrap()
    }

    #[test]
    fn excludes_runtime_and_noise() {
        let rules = rules();
        for path in [
            "/proc/sys/net/ipv4/ip_forward",
            "/sys/class/net/eth0/mtu",
            "/run/systemd/units/invocation:sshd.service",
            "/tmp/build/output",
            "/var/log/messages",
            "/var/cache/dnf/appstream.solv",
            "/var/lib/rpm/rpmdb.sqlite",
            "/var/lib/containers/storage/overlay/abc/diff",
            "/root/.bash_history",
            "/etc/nginx/.nginx.conf.swp",
            "/etc/nginx/nginx.conf~",
            "/opt/app/.git/index",
            "/opt/app/__pycache__/main.cpython-39.pyc",
        ] {
            assert!(rules.is_excluded(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn keeps_what_the_work_touched() {
        let rules = rules();
        for path in [
            "/etc/nginx/nginx.conf",
            "/opt/app/config.yaml",
            "/usr/local/bin/deploy",
            "/srv/data/seed.sql",
            "/root/.ssh/authorized_keys",
        ] {
            assert!(!rules.is_excluded(Path::new(path)), "{path}");
        }
    }

    /// Watching its own data directory would make exarare report itself.
    #[test]
    fn excludes_its_own_data_directory() {
        let rules = rules();
        assert!(rules.is_excluded(Path::new(
            "/var/lib/exarare/sessions/20260913-000000-0001/events.jsonl"
        )));
        assert!(!rules.is_excluded(Path::new("/var/lib/exarare-other/file")));
    }

    #[test]
    fn remembers_the_first_time_a_path_was_seen() {
        let mut touched = Touched::default();
        let early = OffsetDateTime::UNIX_EPOCH;
        let late = early + time::Duration::seconds(60);
        record(&mut touched, PathBuf::from("/opt/a"), early);
        record(&mut touched, PathBuf::from("/opt/a"), late);
        assert_eq!(touched.paths[Path::new("/opt/a")], early);
    }

    #[test]
    fn stops_at_the_cap_and_says_so() {
        let mut touched = Touched::default();
        let now = OffsetDateTime::UNIX_EPOCH;
        for i in 0..MAX_PATHS + 5 {
            record(&mut touched, PathBuf::from(format!("/opt/file{i}")), now);
        }
        assert_eq!(touched.paths.len(), MAX_PATHS);
        assert!(touched.truncated, "the report must admit the list is cut");
    }

    #[test]
    fn selects_paths_by_when_they_were_seen() {
        let mut touched = Touched::default();
        let base = OffsetDateTime::UNIX_EPOCH;
        let late = base + time::Duration::seconds(120);
        record(&mut touched, PathBuf::from("/opt/early"), base);
        record(&mut touched, PathBuf::from("/opt/late"), late);

        // A window ends where the next one starts, so its upper bound is
        // exclusive and a path seen exactly then belongs to the next window.
        let first = touched.in_window(base, Some(late));
        assert_eq!(first, vec![&PathBuf::from("/opt/early")]);

        // The last step has no next one, so its window has no end.
        let last = touched.in_window(late, None);
        assert_eq!(last, vec![&PathBuf::from("/opt/late")]);
    }

    /// The reason the window runs to the next step rather than to the step's own
    /// last command: a watcher reports late, FSEvents especially so, and the
    /// write still happened during that step's work.
    #[test]
    fn a_late_notification_still_lands_in_its_step() {
        let mut touched = Touched::default();
        let step_started = OffsetDateTime::UNIX_EPOCH;
        let last_command = step_started + time::Duration::seconds(1);
        let notified = last_command + time::Duration::seconds(3);
        record(&mut touched, PathBuf::from("/opt/app.yaml"), notified);

        assert!(
            touched.in_window(step_started, None).len() == 1,
            "a delayed event must not fall outside the step that caused it"
        );
    }

    #[test]
    fn round_trips_through_json() {
        let tmp = tempfile::tempdir().unwrap();
        let mut touched = Touched {
            roots: vec![PathBuf::from("/opt")],
            truncated: true,
            ..Default::default()
        };
        record(
            &mut touched,
            PathBuf::from("/opt/app/config.yaml"),
            OffsetDateTime::UNIX_EPOCH,
        );

        let path = tmp.path().join("touched.json");
        save(&touched, &path).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(back.paths.len(), 1);
        assert!(back.truncated);
        assert_eq!(back.roots, vec![PathBuf::from("/opt")]);
    }
}
