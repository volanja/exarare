use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::macros::format_description;

use crate::event::{Event, EventKind};

/// Set in the recording shell so that hooks and subcommands find the session.
pub const ENV_SESSION_DIR: &str = "EXARARE_SESSION_DIR";

const META_FILE: &str = "meta.json";
const EVENTS_FILE: &str = "events.jsonl";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Meta {
    pub id: String,
    pub name: Option<String>,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub shell: String,
    /// Directories snapshotted before and after the session, so their content
    /// is diffed.
    #[serde(default = "default_snapshot_roots")]
    pub snapshot_roots: Vec<PathBuf>,
    /// Directories watched for touches only, with no content recorded.
    #[serde(default)]
    pub watch_roots: Vec<PathBuf>,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option", default)]
    pub ended_at: Option<OffsetDateTime>,
    pub shell_pid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Recording,
    Finished,
    /// Never finished, and the shell is gone (crash, kill -9, reboot).
    Aborted,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Status::Recording => "recording",
            Status::Finished => "finished",
            Status::Aborted => "aborted",
        })
    }
}

pub fn default_snapshot_roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/etc")]
}

/// Root of all exarare data. `EXARARE_HOME` overrides the XDG location.
pub fn data_dir() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("EXARARE_HOME") {
        return Ok(PathBuf::from(p));
    }
    if let Some(p) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(p).join("exarare"));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".local/share/exarare"))
}

fn sessions_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("sessions"))
}

/// Recorded logs may contain secrets, so everything is private to the owner.
fn create_private_dir(path: &Path) -> Result<()> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .with_context(|| format!("failed to create {}", path.display()))
}

pub struct Session {
    pub dir: PathBuf,
    pub meta: Meta,
}

impl Session {
    pub fn create(
        name: Option<String>,
        shell: &Path,
        snapshot_roots: Vec<PathBuf>,
        watch_roots: Vec<PathBuf>,
    ) -> Result<Self> {
        let now = OffsetDateTime::now_utc();
        let stamp = now.format(format_description!(
            "[year][month][day]-[hour][minute][second]"
        ))?;
        let id = format!("{stamp}-{:04x}", std::process::id() & 0xffff);
        let dir = sessions_dir()?.join(&id);
        create_private_dir(&dir)?;

        let meta = Meta {
            id,
            name,
            hostname: nix::unistd::gethostname()
                .ok()
                .and_then(|h| h.into_string().ok()),
            user: std::env::var("USER").ok(),
            shell: shell.display().to_string(),
            snapshot_roots,
            watch_roots,
            started_at: now,
            ended_at: None,
            shell_pid: None,
        };
        let session = Self { dir, meta };
        session.save_meta()?;
        Ok(session)
    }

    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join(META_FILE);
        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let meta = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            meta,
        })
    }

    /// The session of the recording shell we are running in, if any.
    pub fn current() -> Result<Option<Self>> {
        match std::env::var_os(ENV_SESSION_DIR) {
            Some(dir) => Self::open(Path::new(&dir)).map(Some),
            None => Ok(None),
        }
    }

    /// All sessions, oldest first.
    pub fn list() -> Result<Vec<Self>> {
        let root = sessions_dir()?;
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("failed to read {}", root.display())),
        };
        let mut sessions = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.join(META_FILE).is_file() {
                sessions.push(Self::open(&path)?);
            }
        }
        sessions.sort_by(|a, b| a.meta.id.cmp(&b.meta.id));
        Ok(sessions)
    }

    pub fn save_meta(&self) -> Result<()> {
        let path = self.dir.join(META_FILE);
        let tmp = self.dir.join(format!("{META_FILE}.tmp"));
        fs::write(&tmp, serde_json::to_string_pretty(&self.meta)? + "\n")?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Path of the `before` or `after` snapshot.
    pub fn snapshot_path(&self, which: &str) -> PathBuf {
        self.dir.join("snapshots").join(format!("{which}.jsonl"))
    }

    pub fn blobs_dir(&self) -> PathBuf {
        self.dir.join("blobs")
    }

    /// Path of the `before` or `after` package state.
    pub fn packages_path(&self, which: &str) -> PathBuf {
        self.dir.join("packages").join(format!("{which}.json"))
    }

    /// Path of the `before` or `after` service, firewall and account state.
    pub fn state_path(&self, which: &str) -> PathBuf {
        self.dir.join("state").join(format!("{which}.json"))
    }

    /// Path of the set of files the watcher saw being touched.
    pub fn touched_path(&self) -> PathBuf {
        self.dir.join("touched.json")
    }

    pub fn append(&self, kind: EventKind) -> Result<()> {
        append_event(&self.dir, &Event::now(kind))
    }

    pub fn events(&self) -> Result<Vec<Event>> {
        let path = self.dir.join(EVENTS_FILE);
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("failed to open {}", path.display())),
        };
        let mut events = Vec::new();
        for (i, line) in BufReader::new(file).lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            events.push(
                serde_json::from_str(&line)
                    .with_context(|| format!("{}:{}: invalid event", path.display(), i + 1))?,
            );
        }
        Ok(events)
    }

    pub fn status(&self) -> Status {
        if self.meta.ended_at.is_some() {
            return Status::Finished;
        }
        // The pid is written just after the shell is spawned, so a session
        // without one has only just started; the shell may already be running
        // commands. Treating that as aborted would reject the first `exarare
        // note` of a session.
        let Some(pid) = self.meta.shell_pid else {
            return Status::Recording;
        };
        if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok() {
            Status::Recording
        } else {
            Status::Aborted
        }
    }
}

/// Appends without reading `meta.json`, because hooks run on every command.
pub fn append_event(dir: &Path, event: &Event) -> Result<()> {
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(dir.join(EVENTS_FILE))?;
    // A single write keeps concurrent appends from interleaving.
    file.write_all(line.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(shell_pid: Option<u32>, ended: bool) -> Session {
        let now = OffsetDateTime::now_utc();
        Session {
            dir: PathBuf::from("/nonexistent"),
            meta: Meta {
                id: "20260912-000000-0001".into(),
                name: None,
                hostname: None,
                user: None,
                shell: "/bin/bash".into(),
                snapshot_roots: default_snapshot_roots(),
                watch_roots: Vec::new(),
                started_at: now,
                ended_at: ended.then_some(now),
                shell_pid,
            },
        }
    }

    #[test]
    fn a_session_without_a_pid_is_still_recording() {
        // Racing the parent, which writes the pid just after spawning the shell.
        assert_eq!(session(None, false).status(), Status::Recording);
    }

    #[test]
    fn status_follows_the_shell() {
        assert_eq!(
            session(Some(std::process::id()), false).status(),
            Status::Recording
        );
        assert_eq!(
            session(Some(std::process::id()), true).status(),
            Status::Finished
        );
        // Above pid_max, so no such process exists. Pid 1 would not do: tests
        // run as root in the EL containers, where signalling init succeeds.
        assert_eq!(
            session(Some(i32::MAX as u32), false).status(),
            Status::Aborted
        );
    }
}
