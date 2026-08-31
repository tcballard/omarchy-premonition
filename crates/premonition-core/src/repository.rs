//! Explicit repository allowlisting, canonical identity, operation-state, and
//! path-boundary validation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::diff::{ChangeKind, FileChange};
use crate::git::{GitBinary, GitError};
use crate::limits::{MAX_CONFIG_BYTES, MAX_REPOSITORIES, MAX_RESULT_FILE_BYTES};

const CONFIG_VERSION: u16 = 1;

/// Loaded and canonicalized repository allowlist.
#[derive(Clone)]
pub struct RepositoryCatalog {
    git: GitBinary,
    entries: BTreeMap<String, RepositoryEntry>,
}

impl RepositoryCatalog {
    /// Loads a bounded, non-symlink TOML allowlist.
    ///
    /// # Errors
    ///
    /// Returns a stable [`RepositoryError`] for unsafe config metadata,
    /// malformed TOML, duplicate/invalid entries, unsafe labels, or an unsafe
    /// Git binary/repository root.
    pub fn load(path: &Path) -> Result<Self, RepositoryError> {
        let metadata = fs::symlink_metadata(path).map_err(|_| RepositoryError::Config)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_CONFIG_BYTES
        {
            return Err(RepositoryError::Config);
        }
        let mut file = File::open(path).map_err(|_| RepositoryError::Config)?;
        let mut bytes = Vec::with_capacity(
            usize::try_from(metadata.len()).map_err(|_| RepositoryError::Config)?,
        );
        file.by_ref()
            .take(MAX_CONFIG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| RepositoryError::Config)?;
        if u64::try_from(bytes.len()).map_err(|_| RepositoryError::Config)? > MAX_CONFIG_BYTES {
            return Err(RepositoryError::Config);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| RepositoryError::Config)?;
        let config: RawConfig = toml::from_str(text).map_err(|_| RepositoryError::Config)?;
        Self::from_raw(config)
    }

    fn from_raw(config: RawConfig) -> Result<Self, RepositoryError> {
        if config.version != CONFIG_VERSION
            || config.repositories.is_empty()
            || config.repositories.len() > MAX_REPOSITORIES
        {
            return Err(RepositoryError::Config);
        }
        let git = GitBinary::resolve(&config.git_binary)?;
        let mut entries = BTreeMap::new();
        let mut roots = BTreeSet::new();
        for raw in config.repositories {
            validate_id(&raw.id)?;
            validate_label(&raw.label)?;
            if !raw.path.is_absolute() {
                return Err(RepositoryError::Config);
            }
            let root = fs::canonicalize(&raw.path).map_err(|_| RepositoryError::Unsafe)?;
            if !fs::metadata(&root)
                .map_err(|_| RepositoryError::Unsafe)?
                .is_dir()
                || !roots.insert(root.clone())
            {
                return Err(RepositoryError::Config);
            }
            let entry = RepositoryEntry {
                id: raw.id.clone(),
                label: raw.label,
                root,
            };
            if entries.insert(raw.id, entry).is_some() {
                return Err(RepositoryError::Config);
            }
        }
        let catalog = Self { git, entries };
        for id in catalog.entries.keys() {
            let _ = catalog.resolve(id)?;
        }
        Ok(catalog)
    }

    /// Resolves and revalidates one exact allowlist identifier.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryError::Unknown`] for an absent ID and a stable
    /// unsafe-state error if filesystem or Git identity is invalid.
    pub fn resolve(&self, id: &str) -> Result<ResolvedRepository, RepositoryError> {
        let entry = self.entries.get(id).ok_or(RepositoryError::Unknown)?;
        let identity = resolve_identity(&self.git, &entry.root)?;
        Ok(ResolvedRepository {
            id: entry.id.clone(),
            label: entry.label.clone(),
            root: identity.root.clone(),
            git_dir: identity.git_dir.clone(),
            identity,
            git: self.git.clone(),
        })
    }

    /// Returns content-free safe IDs and labels for the UI.
    #[must_use]
    pub fn summaries(&self) -> Vec<(String, String)> {
        self.entries
            .values()
            .map(|entry| (entry.id.clone(), entry.label.clone()))
            .collect()
    }
}

impl fmt::Debug for RepositoryCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepositoryCatalog")
            .field("repositories", &self.entries.len())
            .finish_non_exhaustive()
    }
}

/// Revalidated repository handle used by safety-core operations.
#[derive(Clone)]
pub struct ResolvedRepository {
    id: String,
    label: String,
    root: PathBuf,
    git_dir: PathBuf,
    identity: RepositoryIdentity,
    git: GitBinary,
}

impl ResolvedRepository {
    /// Returns the safe allowlist ID.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the safe UI label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the canonical repository root for internal operations.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the canonical Git directory for internal state checks.
    #[must_use]
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// Returns immutable filesystem/Git identity.
    #[must_use]
    pub fn identity(&self) -> &RepositoryIdentity {
        &self.identity
    }

    /// Returns the canonical Git invocation boundary.
    #[must_use]
    pub fn git(&self) -> &GitBinary {
        &self.git
    }

    /// Re-resolves identity and requires an exact match.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryError::IdentityChanged`] when the repository root
    /// or Git directory has been replaced or retargeted.
    pub fn revalidate_identity(&self) -> Result<(), RepositoryError> {
        let current = resolve_identity(&self.git, &self.root)?;
        if current != self.identity {
            return Err(RepositoryError::IdentityChanged);
        }
        Ok(())
    }

    /// Rejects merge/rebase/cherry-pick/bisect and pre-existing lock state.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryError::OperationInProgress`] when Git state makes
    /// validation or publication ambiguous.
    pub fn assert_operation_safe(&self) -> Result<(), RepositoryError> {
        self.revalidate_identity()?;
        for marker in [
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "BISECT_LOG",
            "index.lock",
            "rebase-apply",
            "rebase-merge",
            "sequencer",
        ] {
            if fs::symlink_metadata(self.git_dir.join(marker)).is_ok() {
                return Err(RepositoryError::OperationInProgress);
            }
        }
        Ok(())
    }

    /// Validates a parsed patch path against the live filesystem.
    ///
    /// # Errors
    ///
    /// Returns a stable error for symlinks, nested Git boundaries, unsafe file
    /// types, hardlinks, missing/existing targets inconsistent with the change,
    /// or oversized files.
    pub fn validate_change_path(&self, change: &FileChange) -> Result<(), RepositoryError> {
        self.revalidate_identity()?;
        let mut current = self.root.clone();
        let components: Vec<_> = change.path.as_path().components().collect();
        if components.is_empty() {
            return Err(RepositoryError::Path);
        }
        for component in &components[..components.len() - 1] {
            let Component::Normal(name) = component else {
                return Err(RepositoryError::Path);
            };
            current.push(name);
            let metadata = fs::symlink_metadata(&current).map_err(|_| RepositoryError::Path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(RepositoryError::Path);
            }
            if fs::symlink_metadata(current.join(".git")).is_ok() {
                return Err(RepositoryError::NestedRepository);
            }
        }
        let target = self.root.join(change.path.as_path());
        match fs::symlink_metadata(&target) {
            Ok(metadata) => {
                if change.kind == ChangeKind::Add
                    || metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || metadata.mode() & 0o777 != 0o644
                    || metadata.nlink() > 1
                    || metadata.len() > MAX_RESULT_FILE_BYTES
                {
                    return Err(RepositoryError::Path);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if change.kind != ChangeKind::Add {
                    return Err(RepositoryError::Path);
                }
            }
            Err(_) => return Err(RepositoryError::Path),
        }
        Ok(())
    }
}

impl fmt::Debug for ResolvedRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedRepository")
            .field("id", &self.id)
            .field("root", &"<redacted>")
            .field("git_dir", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// Filesystem identity bound into every proposal.
#[derive(Clone, Eq, PartialEq)]
pub struct RepositoryIdentity {
    root: PathBuf,
    git_dir: PathBuf,
    root_device: u64,
    root_inode: u64,
    git_device: u64,
    git_inode: u64,
}

impl fmt::Debug for RepositoryIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepositoryIdentity")
            .field("root", &"<redacted>")
            .field("git_dir", &"<redacted>")
            .field("root_device", &self.root_device)
            .field("root_inode", &self.root_inode)
            .field("git_device", &self.git_device)
            .field("git_inode", &self.git_inode)
            .finish()
    }
}

#[derive(Clone)]
struct RepositoryEntry {
    id: String,
    label: String,
    root: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    version: u16,
    git_binary: PathBuf,
    repositories: Vec<RawRepository>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRepository {
    id: String,
    label: String,
    path: PathBuf,
}

/// Stable repository-boundary failures without content-bearing paths.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum RepositoryError {
    /// Configuration file or entry is invalid.
    #[error("repository configuration is invalid")]
    Config,
    /// Repository ID is not allowlisted.
    #[error("repository is not allowlisted")]
    Unknown,
    /// Root/Git layout is unsafe or unsupported.
    #[error("repository is unsafe")]
    Unsafe,
    /// Root or Git directory identity changed.
    #[error("repository identity changed")]
    IdentityChanged,
    /// Git operation/lock state is in progress.
    #[error("repository operation is in progress")]
    OperationInProgress,
    /// Target path/file type is unsafe.
    #[error("repository path is unsafe")]
    Path,
    /// Target path crosses a nested repository boundary.
    #[error("nested repository boundary is unsupported")]
    NestedRepository,
    /// Canonical Git invocation failed.
    #[error("Git repository check failed")]
    Git,
}

impl From<GitError> for RepositoryError {
    fn from(_: GitError) -> Self {
        Self::Git
    }
}

fn resolve_identity(
    git: &GitBinary,
    configured_root: &Path,
) -> Result<RepositoryIdentity, RepositoryError> {
    let root = fs::canonicalize(configured_root).map_err(|_| RepositoryError::Unsafe)?;
    let root_metadata = fs::metadata(&root).map_err(|_| RepositoryError::Unsafe)?;
    if !root_metadata.is_dir() {
        return Err(RepositoryError::Unsafe);
    }
    let dot_git = fs::symlink_metadata(root.join(".git")).map_err(|_| RepositoryError::Unsafe)?;
    if dot_git.file_type().is_symlink() || !(dot_git.is_dir() || dot_git.is_file()) {
        return Err(RepositoryError::Unsafe);
    }

    let top = utf8_trimmed(git.run(&root, &["rev-parse", "--show-toplevel"], None)?)?;
    let top = fs::canonicalize(top).map_err(|_| RepositoryError::Unsafe)?;
    if top != root {
        return Err(RepositoryError::Unsafe);
    }
    if utf8_trimmed(git.run(&root, &["rev-parse", "--is-bare-repository"], None)?)? != "false"
        || utf8_trimmed(git.run(&root, &["rev-parse", "--is-inside-work-tree"], None)?)? != "true"
    {
        return Err(RepositoryError::Unsafe);
    }
    if !utf8_trimmed(git.run(
        &root,
        &["rev-parse", "--show-superproject-working-tree"],
        None,
    )?)?
    .is_empty()
    {
        return Err(RepositoryError::Unsafe);
    }

    let git_dir_value = utf8_trimmed(git.run(&root, &["rev-parse", "--git-dir"], None)?)?;
    let git_dir_candidate = Path::new(&git_dir_value);
    let git_dir = if git_dir_candidate.is_absolute() {
        fs::canonicalize(git_dir_candidate)
    } else {
        fs::canonicalize(root.join(git_dir_candidate))
    }
    .map_err(|_| RepositoryError::Unsafe)?;
    let git_metadata = fs::metadata(&git_dir).map_err(|_| RepositoryError::Unsafe)?;
    if !git_metadata.is_dir() {
        return Err(RepositoryError::Unsafe);
    }
    Ok(RepositoryIdentity {
        root,
        git_dir,
        root_device: root_metadata.dev(),
        root_inode: root_metadata.ino(),
        git_device: git_metadata.dev(),
        git_inode: git_metadata.ino(),
    })
}

fn validate_id(value: &str) -> Result<(), RepositoryError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(RepositoryError::Config);
    }
    Ok(())
}

fn validate_label(value: &str) -> Result<(), RepositoryError> {
    if value.is_empty()
        || value.len() > 80
        || value.chars().any(|character| {
            (character.is_control() && character != '\t')
                || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
    {
        return Err(RepositoryError::Config);
    }
    Ok(())
}

fn utf8_trimmed(bytes: Vec<u8>) -> Result<String, RepositoryError> {
    let value = String::from_utf8(bytes).map_err(|_| RepositoryError::Git)?;
    Ok(value.trim_end_matches(['\r', '\n']).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SafePath;

    #[test]
    fn debug_redacts_repository_paths() {
        let identity = RepositoryIdentity {
            root: PathBuf::from("/unique/secret/repo"),
            git_dir: PathBuf::from("/unique/secret/repo/.git"),
            root_device: 1,
            root_inode: 2,
            git_device: 1,
            git_inode: 3,
        };
        let debug = format!("{identity:?}");
        assert!(!debug.contains("/unique/secret"));
    }

    #[test]
    fn ids_and_labels_are_bounded_and_plain() {
        assert!(validate_id("repo.one").is_ok());
        assert!(validate_id("../repo").is_err());
        assert!(validate_label("Project one").is_ok());
        assert!(validate_label("bad\u{202e}label").is_err());
    }

    #[test]
    fn safe_path_component_contract_is_shared() {
        let safe = SafePath::parse("src/main.rs");
        assert!(safe.is_ok());
        assert!(SafePath::parse("nested/.git/config").is_err());
    }
}
