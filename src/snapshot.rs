//! Snapshots of the watched directories, taken before and after a session.
//!
//! An entry records the metadata and a hash of every file. The content of text
//! files is kept in a content-addressed blob store so that diffs can be shown
//! later.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

/// Text files up to this size get their content stored.
pub const MAX_BLOB_BYTES: u64 = 1 << 20;

/// Files that change on their own, or that are rewritten by the system.
const EXCLUDE: &[&str] = &[
    "/etc/ld.so.cache",
    "/etc/mtab",
    "/etc/machine-id",
    "/etc/.updated",
    "/etc/.pwd.lock",
    "/etc/passwd-",
    "/etc/group-",
    "/etc/shadow-",
    "/etc/gshadow-",
    "/etc/subuid-",
    "/etc/subgid-",
    "/etc/aliases.db",
    "/etc/udev/hwdb.bin",
    "**/*.lock",
    "**/*.swp",
    "**/*~",
];

/// Files whose change is worth recording but whose content is not.
const MASK: &[&str] = &[
    "/etc/shadow",
    "/etc/gshadow",
    "/etc/security/opasswd",
    "**/ssh_host_*_key",
    "**/*.key",
    "**/*.pem",
    // Anchored at /etc, because macOS resolves temporary and system paths
    // under /private, which an unanchored `**/private/**` would match.
    "/etc/**/private/**",
    "/etc/ssl/private/**",
    "**/.ssh/id_*",
];

pub struct Rules {
    exclude: GlobSet,
    mask: GlobSet,
}

fn glob_set(patterns: &[&str]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).with_context(|| format!("bad pattern {pattern}"))?);
    }
    Ok(builder.build()?)
}

impl Rules {
    pub fn defaults() -> Result<Self> {
        Ok(Self {
            exclude: glob_set(EXCLUDE)?,
            mask: glob_set(MASK)?,
        })
    }

    pub fn is_excluded(&self, path: &Path) -> bool {
        self.exclude.is_match(path)
    }

    pub fn is_masked(&self, path: &Path) -> bool {
        self.mask.is_match(path)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    #[serde(flatten)]
    pub content: Content,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    File {
        size: u64,
        hash: String,
        /// Whether the content is in the blob store.
        #[serde(default)]
        blob: bool,
        /// Set for files whose content is deliberately not recorded.
        #[serde(default)]
        masked: bool,
    },
    Symlink {
        target: PathBuf,
    },
}

impl Entry {
    /// True when the file content (or symlink target) differs.
    pub fn content_differs(&self, other: &Entry) -> bool {
        match (&self.content, &other.content) {
            (Content::File { hash: a, .. }, Content::File { hash: b, .. }) => a != b,
            (Content::Symlink { target: a }, Content::Symlink { target: b }) => a != b,
            _ => true,
        }
    }

    pub fn metadata_differs(&self, other: &Entry) -> bool {
        self.mode != other.mode || self.uid != other.uid || self.gid != other.gid
    }

    pub fn hash(&self) -> Option<&str> {
        match &self.content {
            Content::File { hash, .. } => Some(hash),
            Content::Symlink { .. } => None,
        }
    }

    pub fn has_blob(&self) -> bool {
        matches!(self.content, Content::File { blob: true, .. })
    }

    pub fn is_masked(&self) -> bool {
        matches!(self.content, Content::File { masked: true, .. })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub files: usize,
    /// Paths that could not be read (permissions, races).
    pub unreadable: usize,
}

/// Walks `roots` and writes one JSON entry per line to `out`.
pub fn capture(roots: &[PathBuf], out: &Path, blobs: &Path, rules: &Rules) -> Result<Stats> {
    fs::create_dir_all(blobs)?;
    let mut writer = BufWriter::new(File::create(out)?);
    let mut stats = Stats::default();

    for root in roots {
        // /etc is a symlink to /private/etc on macOS.
        let root = match root.canonicalize() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let walk = WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !rules.is_excluded(e.path()));
        for entry in walk {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    stats.unreadable += 1;
                    continue;
                }
            };
            let file_type = entry.file_type();
            if file_type.is_dir() {
                continue;
            }
            if !file_type.is_file() && !file_type.is_symlink() {
                // Sockets, fifos and device nodes carry no content to diff.
                continue;
            }
            match read_entry(entry.path(), blobs, rules) {
                Ok(e) => {
                    writeln!(writer, "{}", serde_json::to_string(&e)?)?;
                    stats.files += 1;
                }
                Err(_) => stats.unreadable += 1,
            }
        }
    }
    writer.flush()?;
    Ok(stats)
}

fn read_entry(path: &Path, blobs: &Path, rules: &Rules) -> Result<Entry> {
    let meta = fs::symlink_metadata(path)?;
    let content = if meta.is_symlink() {
        Content::Symlink {
            target: fs::read_link(path)?,
        }
    } else {
        let masked = rules.is_masked(path);
        let size = meta.len();
        let (hash, body) = hash_file(path, size)?;
        let blob = match &body {
            Some(bytes) if !masked && is_text(bytes) => {
                store_blob(blobs, &hash, bytes)?;
                true
            }
            _ => false,
        };
        Content::File {
            size,
            hash,
            blob,
            masked,
        }
    };
    Ok(Entry {
        path: path.to_path_buf(),
        content,
        mode: meta.mode() & 0o7777,
        uid: meta.uid(),
        gid: meta.gid(),
    })
}

/// Returns the hash and, for small files, the content that was read.
fn hash_file(path: &Path, size: u64) -> Result<(String, Option<Vec<u8>>)> {
    let mut file = File::open(path)?;
    if size <= MAX_BLOB_BYTES {
        let mut bytes = Vec::with_capacity(size as usize);
        file.read_to_end(&mut bytes)?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        return Ok((hash, Some(bytes)));
    }
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok((hasher.finalize().to_hex().to_string(), None))
}

fn is_text(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(8192)];
    !head.contains(&0) && std::str::from_utf8(head).is_ok()
}

fn store_blob(blobs: &Path, hash: &str, bytes: &[u8]) -> Result<()> {
    let path = blobs.join(hash);
    if path.exists() {
        return Ok(());
    }
    let tmp = blobs.join(format!("{hash}.tmp"));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn load(path: &Path) -> Result<BTreeMap<PathBuf, Entry>> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut entries = BTreeMap::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: Entry = serde_json::from_str(&line)
            .with_context(|| format!("{}: invalid snapshot entry", path.display()))?;
        entries.insert(entry.path.clone(), entry);
    }
    Ok(entries)
}

/// Reads a blob back as text, if it was stored.
pub fn blob_text(blobs: &Path, entry: &Entry) -> Option<String> {
    if !entry.has_blob() {
        return None;
    }
    let hash = entry.hash()?;
    fs::read_to_string(blobs.join(hash)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_and_reloads() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("a.conf"), "alpha\n").unwrap();
        fs::write(root.join("binary"), [0u8, 1, 2]).unwrap();
        fs::write(root.join("stale.lock"), "x").unwrap();
        std::os::unix::fs::symlink("a.conf", root.join("link")).unwrap();

        let out = tmp.path().join("before.jsonl");
        let blobs = tmp.path().join("blobs");
        let rules = Rules::defaults().unwrap();
        let stats = capture(std::slice::from_ref(&root), &out, &blobs, &rules).unwrap();
        assert_eq!(stats.files, 3, "the .lock file should be excluded");

        let entries = load(&out).unwrap();
        let root = root.canonicalize().unwrap();
        let text = entries.get(&root.join("a.conf")).unwrap();
        assert_eq!(blob_text(&blobs, text).as_deref(), Some("alpha\n"));
        let binary = entries.get(&root.join("binary")).unwrap();
        assert!(!binary.has_blob(), "binary content is not stored");
        assert!(matches!(
            entries.get(&root.join("link")).unwrap().content,
            Content::Symlink { .. }
        ));
    }

    #[test]
    fn masks_secrets() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("etc");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("service.key"), "secret\n").unwrap();
        fs::write(root.join("server.pem"), "secret\n").unwrap();
        fs::write(root.join("ssh_host_ed25519_key"), "secret\n").unwrap();

        let out = tmp.path().join("s.jsonl");
        let blobs = tmp.path().join("blobs");
        let rules = Rules::defaults().unwrap();
        capture(std::slice::from_ref(&root), &out, &blobs, &rules).unwrap();

        let entries = load(&out).unwrap();
        assert_eq!(entries.len(), 3);
        for entry in entries.values() {
            assert!(
                entry.is_masked(),
                "{} should be masked",
                entry.path.display()
            );
            assert!(!entry.has_blob());
        }
        // Nothing secret reached the blob store.
        assert!(std::fs::read_dir(&blobs).unwrap().next().is_none());
    }

    #[test]
    fn mask_patterns_cover_system_paths() {
        let rules = Rules::defaults().unwrap();
        for path in [
            "/etc/shadow",
            "/etc/gshadow",
            "/etc/pki/tls/private/server.crt",
            "/etc/ssl/private/site.crt",
            "/etc/ssh/ssh_host_rsa_key",
            "/root/.ssh/id_ed25519",
        ] {
            assert!(rules.is_masked(Path::new(path)), "{path} should be masked");
        }
        for path in ["/etc/hosts", "/etc/nginx/nginx.conf", "/etc/passwd"] {
            assert!(
                !rules.is_masked(Path::new(path)),
                "{path} should not be masked"
            );
        }
        // A temporary directory on macOS lives under /private but holds no secrets.
        assert!(!rules.is_masked(Path::new("/private/var/folders/x/T/tmp1/a.conf")));
    }
}
