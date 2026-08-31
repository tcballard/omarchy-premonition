//! Conservative generation-time repository snapshots used to reject any
//! incompatible change before Apply.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::limits::{MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_FILES};
use crate::repository::{RepositoryError, ResolvedRepository};

/// Exact conservative token captured before proposal publication.
#[derive(Clone, Eq, PartialEq)]
pub struct RepositorySnapshot {
    head_oid: String,
    head_ref_digest: [u8; 32],
    index_digest: [u8; 32],
    worktree_digest: [u8; 32],
    file_count: u32,
    total_bytes: u64,
}

impl RepositorySnapshot {
    /// Captures HEAD, branch identity, real index bytes, and bounded tracked /
    /// untracked worktree metadata and contents without following symlinks.
    ///
    /// # Errors
    ///
    /// Returns a stable [`SnapshotError`] when repository identity/state is
    /// unsafe, enumeration is ambiguous/oversized, a special file exists, or a
    /// file changes while being hashed.
    pub fn capture(repository: &ResolvedRepository) -> Result<Self, SnapshotError> {
        repository.assert_operation_safe()?;
        let head_oid = utf8_line(repository.git().run(
            repository.root(),
            &["rev-parse", "--verify", "HEAD^{commit}"],
            None,
        )?)?;
        if !(head_oid.len() == 40 || head_oid.len() == 64)
            || !head_oid.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(SnapshotError::Git);
        }
        let head_ref = repository.git().run(
            repository.root(),
            &["rev-parse", "--abbrev-ref", "HEAD"],
            None,
        )?;
        let head_ref_digest = Sha256::digest(&head_ref).into();
        let index_digest =
            hash_regular_file(&repository.git_dir().join("index"), 64 * 1024 * 1024)?;

        let names = repository.git().run(
            repository.root(),
            &[
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ],
            None,
        )?;
        let paths = parse_nul_paths(&names)?;
        if paths.len() > MAX_SNAPSHOT_FILES {
            return Err(SnapshotError::TooLarge);
        }

        let mut hash = Sha256::new();
        let mut total_bytes = 0_u64;
        for relative in &paths {
            hash.update(b"path\0");
            hash.update(relative.as_os_str().as_bytes());
            hash.update([0]);
            let absolute = repository.root().join(relative);
            match fs::symlink_metadata(&absolute) {
                Ok(metadata) if metadata.is_file() => {
                    total_bytes = total_bytes
                        .checked_add(metadata.len())
                        .ok_or(SnapshotError::TooLarge)?;
                    if total_bytes > MAX_SNAPSHOT_BYTES {
                        return Err(SnapshotError::TooLarge);
                    }
                    hash.update(b"file\0");
                    hash_metadata(&mut hash, &metadata);
                    hash_file_stable(&absolute, &metadata, &mut hash)?;
                }
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    hash.update(b"symlink\0");
                    hash_metadata(&mut hash, &metadata);
                    let target = fs::read_link(&absolute).map_err(|_| SnapshotError::Io)?;
                    hash.update(target.as_os_str().as_bytes());
                }
                Ok(_) => return Err(SnapshotError::UnsafeFileType),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    hash.update(b"missing\0");
                }
                Err(_) => return Err(SnapshotError::Io),
            }
        }

        Ok(Self {
            head_oid,
            head_ref_digest,
            index_digest,
            worktree_digest: hash.finalize().into(),
            file_count: u32::try_from(paths.len()).map_err(|_| SnapshotError::TooLarge)?,
            total_bytes,
        })
    }

    /// Recaptures and requires an exact generation-time match.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError::Stale`] whenever HEAD, branch, index,
    /// worktree metadata/content, or enumeration differs.
    pub fn revalidate(&self, repository: &ResolvedRepository) -> Result<(), SnapshotError> {
        if &Self::capture(repository)? != self {
            return Err(SnapshotError::Stale);
        }
        Ok(())
    }

    /// Returns the captured commit OID.
    #[must_use]
    pub fn head_oid(&self) -> &str {
        &self.head_oid
    }

    /// Returns the bounded enumerated-file count.
    #[must_use]
    pub fn file_count(&self) -> u32 {
        self.file_count
    }
}

impl fmt::Debug for RepositorySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepositorySnapshot")
            .field("head_oid", &self.head_oid)
            .field("files", &self.file_count)
            .field("bytes", &self.total_bytes)
            .finish_non_exhaustive()
    }
}

/// Stable snapshot failures without paths or contents.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SnapshotError {
    /// Repository boundary/state is unsafe.
    #[error("repository is unsafe")]
    Repository,
    /// Git enumeration failed or was ambiguous.
    #[error("repository enumeration failed")]
    Git,
    /// Snapshot file/byte ceiling exceeded.
    #[error("repository snapshot is too large")]
    TooLarge,
    /// FIFO/socket/device or other special path was enumerated.
    #[error("repository contains an unsafe file type")]
    UnsafeFileType,
    /// File changed while being hashed.
    #[error("repository changed during snapshot")]
    Unstable,
    /// Bounded filesystem I/O failed.
    #[error("repository snapshot I/O failed")]
    Io,
    /// Captured token no longer matches.
    #[error("repository proposal is stale")]
    Stale,
}

impl From<RepositoryError> for SnapshotError {
    fn from(_: RepositoryError) -> Self {
        Self::Repository
    }
}

impl From<crate::git::GitError> for SnapshotError {
    fn from(_: crate::git::GitError) -> Self {
        Self::Git
    }
}

fn parse_nul_paths(bytes: &[u8]) -> Result<Vec<PathBuf>, SnapshotError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if bytes.last() != Some(&0) {
        return Err(SnapshotError::Git);
    }
    let mut unique = BTreeSet::new();
    for raw in bytes[..bytes.len() - 1].split(|byte| *byte == 0) {
        if raw.is_empty() || raw.starts_with(b"/") {
            return Err(SnapshotError::Git);
        }
        let path = PathBuf::from(std::ffi::OsString::from_vec(raw.to_vec()));
        for component in path.components() {
            match component {
                Component::Normal(value) if !value.as_bytes().is_empty() => {}
                _ => return Err(SnapshotError::Git),
            }
        }
        if !unique.insert(path) {
            return Err(SnapshotError::Git);
        }
    }
    Ok(unique.into_iter().collect())
}

fn hash_regular_file(path: &Path, limit: u64) -> Result<[u8; 32], SnapshotError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| SnapshotError::Io)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > limit {
        return Err(SnapshotError::Io);
    }
    let mut hash = Sha256::new();
    hash_file_stable(path, &metadata, &mut hash)?;
    Ok(hash.finalize().into())
}

fn hash_file_stable(
    path: &Path,
    before: &Metadata,
    hash: &mut Sha256,
) -> Result<(), SnapshotError> {
    let mut file = File::open(path).map_err(|_| SnapshotError::Io)?;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = file.read(&mut buffer).map_err(|_| SnapshotError::Io)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let after = file.metadata().map_err(|_| SnapshotError::Io)?;
    if !same_metadata(before, &after) {
        return Err(SnapshotError::Unstable);
    }
    Ok(())
}

fn hash_metadata(hash: &mut Sha256, metadata: &Metadata) {
    hash.update(metadata.dev().to_be_bytes());
    hash.update(metadata.ino().to_be_bytes());
    hash.update(metadata.mode().to_be_bytes());
    hash.update(metadata.nlink().to_be_bytes());
    hash.update(metadata.len().to_be_bytes());
    hash.update(metadata.mtime().to_be_bytes());
    hash.update(metadata.mtime_nsec().to_be_bytes());
    hash.update(metadata.ctime().to_be_bytes());
    hash.update(metadata.ctime_nsec().to_be_bytes());
}

fn same_metadata(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.mode() == right.mode()
        && left.nlink() == right.nlink()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

fn utf8_line(bytes: Vec<u8>) -> Result<String, SnapshotError> {
    let value = String::from_utf8(bytes).map_err(|_| SnapshotError::Git)?;
    let trimmed = value.trim_end_matches(['\r', '\n']);
    if trimmed.is_empty() || trimmed.contains('\n') {
        return Err(SnapshotError::Git);
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_path_parser_rejects_traversal_and_missing_terminator() {
        assert!(parse_nul_paths(b"src/main.rs\0").is_ok());
        assert_eq!(parse_nul_paths(b"../secret\0"), Err(SnapshotError::Git));
        assert_eq!(parse_nul_paths(b"src/main.rs"), Err(SnapshotError::Git));
    }
}
