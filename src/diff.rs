//! Compares two snapshots and renders what changed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use similar::TextDiff;

use crate::snapshot::{Entry, blob_text};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added(Entry),
    Removed(Entry),
    Modified {
        before: Entry,
        after: Entry,
    },
    /// Same content, different mode or owner.
    MetadataChanged {
        before: Entry,
        after: Entry,
    },
}

impl Change {
    pub fn path(&self) -> &Path {
        match self {
            Change::Added(e) | Change::Removed(e) => &e.path,
            Change::Modified { after, .. } | Change::MetadataChanged { after, .. } => &after.path,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Change::Added(_) => "added",
            Change::Removed(_) => "removed",
            Change::Modified { .. } => "modified",
            Change::MetadataChanged { .. } => "permissions",
        }
    }
}

pub fn compare(before: &BTreeMap<PathBuf, Entry>, after: &BTreeMap<PathBuf, Entry>) -> Vec<Change> {
    let mut changes = Vec::new();
    for (path, after_entry) in after {
        match before.get(path) {
            None => changes.push(Change::Added(after_entry.clone())),
            Some(before_entry) if before_entry.content_differs(after_entry) => {
                changes.push(Change::Modified {
                    before: before_entry.clone(),
                    after: after_entry.clone(),
                })
            }
            Some(before_entry) if before_entry.metadata_differs(after_entry) => {
                changes.push(Change::MetadataChanged {
                    before: before_entry.clone(),
                    after: after_entry.clone(),
                })
            }
            Some(_) => {}
        }
    }
    for (path, before_entry) in before {
        if !after.contains_key(path) {
            changes.push(Change::Removed(before_entry.clone()));
        }
    }
    changes.sort_by(|a, b| a.path().cmp(b.path()));
    changes
}

/// A human-readable report, used by `exarare diff`.
pub fn render(changes: &[Change], blobs: &Path) -> String {
    if changes.is_empty() {
        return "no file changes\n".to_string();
    }
    let mut out = String::new();
    for change in changes {
        out.push_str(&format!(
            "{:<12} {}\n",
            change.label(),
            change.path().display()
        ));
        match change {
            Change::Added(entry) => {
                if let Some(text) = blob_text(blobs, entry) {
                    out.push_str(&unified(&change.path().display().to_string(), "", &text));
                } else {
                    out.push_str(&note(entry));
                }
            }
            Change::Removed(entry) => {
                if let Some(text) = blob_text(blobs, entry) {
                    out.push_str(&unified(&change.path().display().to_string(), &text, ""));
                } else {
                    out.push_str(&note(entry));
                }
            }
            Change::Modified { before, after } => {
                match (blob_text(blobs, before), blob_text(blobs, after)) {
                    (Some(a), Some(b)) => {
                        out.push_str(&unified(&change.path().display().to_string(), &a, &b))
                    }
                    _ => out.push_str(&note(after)),
                }
            }
            Change::MetadataChanged { before, after } => out.push_str(&format!(
                "  mode {:o} -> {:o}, owner {}:{} -> {}:{}\n",
                before.mode, after.mode, before.uid, before.gid, after.uid, after.gid
            )),
        }
    }
    out
}

fn note(entry: &Entry) -> String {
    if entry.is_masked() {
        "  (content not recorded: the path matched a secret pattern)\n".to_string()
    } else {
        "  (content not recorded: binary or too large)\n".to_string()
    }
}

/// The unified diff of one change, when both sides were recorded as text.
/// `None` for a binary, oversized or masked file, for symlinks, and for a
/// change that is only in the mode or owner.
pub fn unified_body(change: &Change, blobs: &Path) -> Option<String> {
    let path = change.path().display().to_string();
    match change {
        Change::Added(entry) => Some(unified_raw(&path, "", &blob_text(blobs, entry)?)),
        Change::Removed(entry) => Some(unified_raw(&path, &blob_text(blobs, entry)?, "")),
        Change::Modified { before, after } => Some(unified_raw(
            &path,
            &blob_text(blobs, before)?,
            &blob_text(blobs, after)?,
        )),
        Change::MetadataChanged { .. } => None,
    }
}

fn unified_raw(path: &str, before: &str, after: &str) -> String {
    TextDiff::from_lines(before, after)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a{path}"), &format!("b{path}"))
        .to_string()
}

fn unified(path: &str, before: &str, after: &str) -> String {
    unified_raw(path, before, after)
        .lines()
        .map(|l| format!("  {l}\n"))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::Content;

    fn file(path: &str, hash: &str, mode: u32) -> Entry {
        Entry {
            path: PathBuf::from(path),
            content: Content::File {
                size: 1,
                hash: hash.into(),
                blob: false,
                masked: false,
            },
            mode,
            uid: 0,
            gid: 0,
        }
    }

    #[test]
    fn classifies_changes() {
        let before = BTreeMap::from([
            (PathBuf::from("/etc/a"), file("/etc/a", "h1", 0o644)),
            (PathBuf::from("/etc/b"), file("/etc/b", "h2", 0o644)),
            (PathBuf::from("/etc/c"), file("/etc/c", "h3", 0o644)),
        ]);
        let after = BTreeMap::from([
            (PathBuf::from("/etc/a"), file("/etc/a", "h9", 0o644)),
            (PathBuf::from("/etc/b"), file("/etc/b", "h2", 0o600)),
            (PathBuf::from("/etc/d"), file("/etc/d", "h4", 0o644)),
        ]);

        let changes = compare(&before, &after);
        let summary: Vec<_> = changes
            .iter()
            .map(|c| (c.label(), c.path().display().to_string()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("modified", "/etc/a".to_string()),
                ("permissions", "/etc/b".to_string()),
                ("removed", "/etc/c".to_string()),
                ("added", "/etc/d".to_string()),
            ]
        );
    }

    #[test]
    fn renders_unified_diff() {
        let body = unified("/etc/hosts", "one\ntwo\n", "one\nthree\n");
        assert!(body.contains("a/etc/hosts"));
        assert!(body.contains("-two"));
        assert!(body.contains("+three"));
    }
}
