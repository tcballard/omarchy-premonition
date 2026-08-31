//! Fail-closed repository, patch, proposal, and Apply policy.

mod apply;
mod diff;
mod git;
pub mod limits;
mod repository;
mod sensitive;
mod snapshot;

use std::fmt;
use std::path::{Path, PathBuf};

use thiserror::Error;

pub use apply::{ApplyEngine, ApplyError, ApplyOutcome, RecoveryReport};
pub use diff::{ChangeKind, DiffError, FileChange, ParsedPatch, SafePath};
pub use git::{GitBinary, GitError};
pub use repository::{RepositoryCatalog, RepositoryError, RepositoryIdentity, ResolvedRepository};
pub use sensitive::SensitiveText;
pub use snapshot::{RepositorySnapshot, SnapshotError};

/// Wire-compatible core model version.
pub const CORE_MODEL_VERSION: u16 = 1;

/// Main stateless safety-policy entry point.
#[derive(Clone, Debug)]
pub struct SafetyCore {
    catalog: RepositoryCatalog,
}

impl SafetyCore {
    /// Loads and eagerly validates the repository allowlist.
    ///
    /// # Errors
    ///
    /// Returns a stable [`CoreError`] when configuration, Git, or an allowlist
    /// entry is unsafe.
    pub fn load(config_path: &Path) -> Result<Self, CoreError> {
        Ok(Self {
            catalog: RepositoryCatalog::load(config_path)?,
        })
    }

    /// Captures the exact repository token before executor investigation.
    ///
    /// # Errors
    ///
    /// Returns a stable [`CoreError`] when the ID is unknown or the repository
    /// cannot be safely resolved and snapshotted.
    pub fn begin_investigation(&self, repository_id: &str) -> Result<GenerationContext, CoreError> {
        let repository = self.catalog.resolve(repository_id)?;
        repository.assert_operation_safe()?;
        let snapshot = RepositorySnapshot::capture(&repository)?;
        Ok(GenerationContext {
            repository,
            snapshot,
        })
    }

    /// Validates executor output against the exact pre-execution context.
    ///
    /// # Errors
    ///
    /// Returns a stable [`CoreError`] if the repository changed, the patch is
    /// malformed/unsupported, a path is unsafe, or Git rejects applicability.
    pub fn validate_candidate(
        &self,
        context: GenerationContext,
        raw_patch: String,
    ) -> Result<ValidatedProposal, CoreError> {
        let current = self.catalog.resolve(context.repository.id())?;
        if current.identity() != context.repository.identity() {
            return Err(CoreError::Repository(RepositoryError::IdentityChanged));
        }
        context.snapshot.revalidate(&current)?;
        let patch = ParsedPatch::parse(raw_patch)?;
        for change in patch.files() {
            current.validate_change_path(change)?;
        }
        git_apply_check(&current, &patch)?;
        context.snapshot.revalidate(&current)?;
        Ok(ValidatedProposal {
            repository: current,
            snapshot: context.snapshot,
            patch,
        })
    }

    /// Returns content-free repository IDs/labels.
    #[must_use]
    pub fn repository_summaries(&self) -> Vec<(String, String)> {
        self.catalog.summaries()
    }
}

/// Pre-executor repository identity and conservative snapshot.
#[derive(Clone)]
pub struct GenerationContext {
    repository: ResolvedRepository,
    snapshot: RepositorySnapshot,
}

impl GenerationContext {
    /// Canonical repository root passed only to the bounded executor.
    #[must_use]
    pub fn repository_root(&self) -> &Path {
        self.repository.root()
    }

    /// Safe allowlist identifier.
    #[must_use]
    pub fn repository_id(&self) -> &str {
        self.repository.id()
    }

    /// Generation-time snapshot used to prove the executor did not mutate.
    #[must_use]
    pub fn snapshot(&self) -> &RepositorySnapshot {
        &self.snapshot
    }
}

impl fmt::Debug for GenerationContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GenerationContext")
            .field("repository_id", &self.repository.id())
            .field("snapshot", &self.snapshot)
            .finish_non_exhaustive()
    }
}

/// Validated in-memory candidate bound to one exact repository snapshot.
#[derive(Clone)]
pub struct ValidatedProposal {
    pub(crate) repository: ResolvedRepository,
    pub(crate) snapshot: RepositorySnapshot,
    pub(crate) patch: ParsedPatch,
}

impl ValidatedProposal {
    /// Safe allowlist identifier.
    #[must_use]
    pub fn repository_id(&self) -> &str {
        self.repository.id()
    }

    /// Explicitly exposes the validated patch for review/copy/Apply only.
    #[must_use]
    pub fn patch(&self) -> &str {
        self.patch.expose()
    }

    /// Number of changed files.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.patch.files().len()
    }

    /// Patch byte count without exposing content.
    #[must_use]
    pub fn patch_bytes(&self) -> usize {
        self.patch.len()
    }

    /// Captured commit OID.
    #[must_use]
    pub fn head_oid(&self) -> &str {
        self.snapshot.head_oid()
    }
}

impl fmt::Debug for ValidatedProposal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedProposal")
            .field("repository_id", &self.repository.id())
            .field("snapshot", &self.snapshot)
            .field("patch", &self.patch)
            .finish_non_exhaustive()
    }
}

/// Stable safety-core errors containing no source, patch, or paths.
#[derive(Debug, Error)]
pub enum CoreError {
    /// Repository boundary/configuration failure.
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    /// Snapshot/staleness failure.
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    /// Unified-diff failure.
    #[error(transparent)]
    Diff(#[from] DiffError),
    /// Final Git applicability failure.
    #[error("candidate is not applicable")]
    ApplyCheck,
}

fn git_apply_check(repository: &ResolvedRepository, patch: &ParsedPatch) -> Result<(), CoreError> {
    repository
        .git()
        .run(
            repository.root(),
            &["apply", "--check", "--whitespace=nowarn", "-"],
            Some(patch.expose().as_bytes()),
        )
        .map_err(|_| CoreError::ApplyCheck)?;
    Ok(())
}

pub(crate) fn copy_file_bounded(
    source: &Path,
    destination: &Path,
    max_bytes: u64,
) -> Result<u64, std::io::Error> {
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};

    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut total = 0_u64;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(count).map_err(std::io::Error::other)?)
            .ok_or_else(|| std::io::Error::other("size overflow"))?;
        if total > max_bytes {
            return Err(std::io::Error::other("size limit"));
        }
        output.write_all(&buffer[..count])?;
    }
    output.sync_all()?;
    Ok(total)
}

pub(crate) fn transaction_path(root: &Path, id: &str) -> PathBuf {
    root.join(format!("tx-{id}"))
}
