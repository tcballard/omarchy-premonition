//! Transactional, explicitly invoked proposal publication.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::diff::ChangeKind;
use crate::limits::MAX_RESULT_FILE_BYTES;
use crate::{ValidatedProposal, copy_file_bounded, transaction_path};

const JOURNAL_VERSION: u16 = 1;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024;

/// Explicit Apply implementation with a private durable transaction root.
#[derive(Clone, Debug)]
pub struct ApplyEngine {
    state_root: PathBuf,
}

impl ApplyEngine {
    /// Creates and validates a private state directory.
    ///
    /// # Errors
    ///
    /// Fails if the directory is a symlink, not owned by the current user, or
    /// accessible by group/other users.
    pub fn new(state_root: &Path) -> Result<Self, ApplyError> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let existed = state_root.exists();
        fs::create_dir_all(state_root).map_err(|_| ApplyError::State)?;
        if !existed {
            fs::set_permissions(state_root, fs::Permissions::from_mode(0o700))
                .map_err(|_| ApplyError::State)?;
        }
        let metadata = fs::symlink_metadata(state_root).map_err(|_| ApplyError::State)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.uid() != unsafe_effective_uid()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(ApplyError::State);
        }
        Ok(Self {
            state_root: fs::canonicalize(state_root).map_err(|_| ApplyError::State)?,
        })
    }

    /// Recovers every bounded, recognized transaction before serving calls.
    ///
    /// # Errors
    ///
    /// Fails closed if an unknown entry or unresolvable journal exists.
    pub fn recover(&self) -> Result<RecoveryReport, ApplyError> {
        let mut recovered = 0_u32;
        for entry in fs::read_dir(&self.state_root).map_err(|_| ApplyError::State)? {
            let entry = entry.map_err(|_| ApplyError::State)?;
            let name = entry.file_name();
            let name = name.to_str().ok_or(ApplyError::RecoveryRequired)?;
            if !name.starts_with("tx-")
                || fs::symlink_metadata(entry.path())
                    .map_err(|_| ApplyError::State)?
                    .file_type()
                    .is_symlink()
            {
                return Err(ApplyError::RecoveryRequired);
            }
            Self::recover_one(&entry.path())?;
            recovered = recovered.checked_add(1).ok_or(ApplyError::State)?;
        }
        Ok(RecoveryReport { recovered })
    }

    /// Revalidates and publishes a proposal only after the caller's explicit
    /// Apply action.
    ///
    /// # Errors
    ///
    /// Any identity, staleness, path, applicability, staging, or publication
    /// failure aborts. A failed publication is rolled back; inability to prove
    /// rollback leaves a recovery journal and returns `RecoveryRequired`.
    pub fn apply(
        &self,
        proposal_id: &str,
        proposal: &ValidatedProposal,
    ) -> Result<ApplyOutcome, ApplyError> {
        validate_transaction_id(proposal_id)?;
        proposal.repository.assert_operation_safe()?;
        proposal.snapshot.revalidate(&proposal.repository)?;
        for change in proposal.patch.files() {
            proposal.repository.validate_change_path(change)?;
        }
        proposal.repository.git().run(
            proposal.repository.root(),
            &["apply", "--check", "--whitespace=nowarn", "-"],
            Some(proposal.patch.expose().as_bytes()),
        )?;
        proposal.snapshot.revalidate(&proposal.repository)?;

        let transaction = transaction_path(&self.state_root, proposal_id);
        fs::create_dir(&transaction).map_err(|_| ApplyError::TransactionExists)?;
        let result = Self::prepare_and_publish(&transaction, proposal_id, proposal);
        if result.is_err() && transaction.exists() && Self::recover_one(&transaction).is_err() {
            return Err(ApplyError::RecoveryRequired);
        }
        result
    }

    fn prepare_and_publish(
        transaction: &Path,
        proposal_id: &str,
        proposal: &ValidatedProposal,
    ) -> Result<ApplyOutcome, ApplyError> {
        let stage = transaction.join("stage");
        fs::create_dir(&stage).map_err(|_| ApplyError::State)?;
        let mut entries = Vec::with_capacity(proposal.patch.files().len());
        for (index, change) in proposal.patch.files().iter().enumerate() {
            let source = proposal.repository.root().join(change.path.as_path());
            let staged = stage.join(change.path.as_path());
            if let Some(parent) = staged.parent() {
                fs::create_dir_all(parent).map_err(|_| ApplyError::State)?;
            }
            let pre_hash = match change.kind {
                ChangeKind::Add => None,
                ChangeKind::Modify | ChangeKind::Delete => {
                    copy_file_bounded(&source, &staged, MAX_RESULT_FILE_BYTES)
                        .map_err(|_| ApplyError::State)?;
                    Some(hash_file(&source)?)
                }
            };
            entries.push(JournalEntry {
                relative: change.path.as_str().to_owned(),
                kind: change.kind.into(),
                temporary: format!(".premonition-{proposal_id}-{index}.new"),
                backup: format!(".premonition-{proposal_id}-{index}.bak"),
                pre_hash,
                post_hash: None,
                published: false,
            });
        }

        proposal.repository.git().run(
            &stage,
            &[
                "apply",
                "--check",
                "--unsafe-paths",
                "--whitespace=nowarn",
                "-",
            ],
            Some(proposal.patch.expose().as_bytes()),
        )?;
        proposal.repository.git().run(
            &stage,
            &["apply", "--unsafe-paths", "--whitespace=nowarn", "-"],
            Some(proposal.patch.expose().as_bytes()),
        )?;
        proposal.snapshot.revalidate(&proposal.repository)?;

        for entry in &mut entries {
            let relative = Path::new(&entry.relative);
            let staged = stage.join(relative);
            if entry.kind != JournalKind::Delete {
                entry.post_hash = Some(hash_file(&staged)?);
            }
        }

        let mut journal = Journal {
            version: JOURNAL_VERSION,
            repository_root: proposal.repository.root().to_path_buf(),
            phase: Phase::Prepared,
            entries,
        };
        write_journal(transaction, &journal)?;
        for entry in &journal.entries {
            use std::os::unix::fs::PermissionsExt;

            if entry.kind == JournalKind::Delete {
                continue;
            }
            let relative = Path::new(&entry.relative);
            let target = proposal.repository.root().join(relative);
            let parent = target.parent().ok_or(ApplyError::State)?;
            copy_file_bounded(
                &stage.join(relative),
                &parent.join(&entry.temporary),
                MAX_RESULT_FILE_BYTES,
            )
            .map_err(|_| ApplyError::State)?;
            fs::set_permissions(
                parent.join(&entry.temporary),
                fs::Permissions::from_mode(0o644),
            )
            .map_err(|_| ApplyError::State)?;
        }
        journal.phase = Phase::Publishing;
        write_journal(transaction, &journal)?;

        for index in 0..journal.entries.len() {
            publish_entry(&journal.repository_root, &journal.entries[index])?;
            journal.entries[index].published = true;
            write_journal(transaction, &journal)?;
        }
        journal.phase = Phase::Committed;
        write_journal(transaction, &journal)?;
        sync_directory(&journal.repository_root)?;
        cleanup_transaction(transaction, &journal)?;
        Ok(ApplyOutcome {
            files_changed: u32::try_from(journal.entries.len()).map_err(|_| ApplyError::State)?,
        })
    }

    fn recover_one(transaction: &Path) -> Result<(), ApplyError> {
        let journal_path = transaction.join("journal.json");
        if !journal_path.exists() {
            remove_transaction(transaction)?;
            return Ok(());
        }
        let journal = read_journal(&journal_path)?;
        validate_journal(&journal)?;
        match journal.phase {
            Phase::Prepared => cleanup_transaction(transaction, &journal),
            Phase::Publishing => {
                for entry in journal.entries.iter().rev() {
                    rollback_entry(&journal.repository_root, entry)?;
                }
                cleanup_transaction(transaction, &journal)
            }
            Phase::Committed => {
                for entry in &journal.entries {
                    verify_post(&journal.repository_root, entry)?;
                }
                cleanup_transaction(transaction, &journal)
            }
        }
    }
}

/// Successful Apply summary containing no paths or contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplyOutcome {
    /// Atomically published file count.
    pub files_changed: u32,
}

/// Startup recovery summary containing no paths or contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryReport {
    /// Transactions resolved to a fully pre- or post-Apply state.
    pub recovered: u32,
}

/// Stable publication failures without source, patch, or path content.
#[derive(Debug, Error)]
pub enum ApplyError {
    /// Private state boundary is unsafe.
    #[error("Apply state is unsafe")]
    State,
    /// Transaction identifier is invalid.
    #[error("proposal identifier is invalid")]
    Identifier,
    /// A transaction with this identifier already exists.
    #[error("proposal transaction already exists")]
    TransactionExists,
    /// Repository boundary changed or is unsafe.
    #[error("repository is unsafe")]
    Repository(#[from] crate::RepositoryError),
    /// Generation snapshot is stale.
    #[error("proposal is stale")]
    Snapshot(#[from] crate::SnapshotError),
    /// Git rejected revalidation or staging.
    #[error("Git rejected Apply")]
    Git(#[from] crate::GitError),
    /// Publication could not be proven fully rolled back/forward.
    #[error("Apply recovery is required")]
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    Publishing,
    Committed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum JournalKind {
    Modify,
    Add,
    Delete,
}

impl From<ChangeKind> for JournalKind {
    fn from(value: ChangeKind) -> Self {
        match value {
            ChangeKind::Modify => Self::Modify,
            ChangeKind::Add => Self::Add,
            ChangeKind::Delete => Self::Delete,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u16,
    repository_root: PathBuf,
    phase: Phase,
    entries: Vec<JournalEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JournalEntry {
    relative: String,
    kind: JournalKind,
    temporary: String,
    backup: String,
    pre_hash: Option<String>,
    post_hash: Option<String>,
    published: bool,
}

fn publish_entry(root: &Path, entry: &JournalEntry) -> Result<(), ApplyError> {
    let target = root.join(&entry.relative);
    let parent = target.parent().ok_or(ApplyError::State)?;
    verify_pre(root, entry)?;
    match entry.kind {
        JournalKind::Add => fs::rename(parent.join(&entry.temporary), &target),
        JournalKind::Modify => {
            fs::rename(&target, parent.join(&entry.backup)).map_err(|_| ApplyError::State)?;
            if fs::rename(parent.join(&entry.temporary), &target).is_err() {
                fs::rename(parent.join(&entry.backup), &target)
                    .map_err(|_| ApplyError::RecoveryRequired)?;
                return Err(ApplyError::State);
            }
            Ok(())
        }
        JournalKind::Delete => fs::rename(&target, parent.join(&entry.backup)),
    }
    .map_err(|_| ApplyError::State)?;
    sync_directory(parent)
}

fn rollback_entry(root: &Path, entry: &JournalEntry) -> Result<(), ApplyError> {
    let target = root.join(&entry.relative);
    let parent = target.parent().ok_or(ApplyError::RecoveryRequired)?;
    let backup = parent.join(&entry.backup);
    match entry.kind {
        JournalKind::Add => {
            if target.exists() {
                verify_post(root, entry)?;
                fs::remove_file(&target).map_err(|_| ApplyError::RecoveryRequired)?;
            }
        }
        JournalKind::Modify => {
            if backup.exists() {
                if target.exists() {
                    verify_post(root, entry)?;
                    fs::remove_file(&target).map_err(|_| ApplyError::RecoveryRequired)?;
                }
                fs::rename(&backup, &target).map_err(|_| ApplyError::RecoveryRequired)?;
            }
        }
        JournalKind::Delete => {
            if backup.exists() {
                if target.exists() {
                    return Err(ApplyError::RecoveryRequired);
                }
                fs::rename(&backup, &target).map_err(|_| ApplyError::RecoveryRequired)?;
            }
        }
    }
    sync_directory(parent).map_err(|_| ApplyError::RecoveryRequired)
}

fn verify_pre(root: &Path, entry: &JournalEntry) -> Result<(), ApplyError> {
    let target = root.join(&entry.relative);
    match (&entry.kind, &entry.pre_hash) {
        (JournalKind::Add, None) if !target.exists() => Ok(()),
        (JournalKind::Modify | JournalKind::Delete, Some(expected)) => {
            if hash_file(&target).is_ok_and(|actual| actual == *expected) {
                Ok(())
            } else {
                Err(ApplyError::Snapshot(crate::SnapshotError::Stale))
            }
        }
        _ => Err(ApplyError::Snapshot(crate::SnapshotError::Stale)),
    }
}

fn verify_post(root: &Path, entry: &JournalEntry) -> Result<(), ApplyError> {
    let target = root.join(&entry.relative);
    match (&entry.kind, &entry.post_hash) {
        (JournalKind::Delete, None) if !target.exists() => Ok(()),
        (JournalKind::Add | JournalKind::Modify, Some(expected)) => {
            if hash_file(&target).is_ok_and(|actual| actual == *expected) {
                Ok(())
            } else {
                Err(ApplyError::RecoveryRequired)
            }
        }
        _ => Err(ApplyError::RecoveryRequired),
    }
}

fn validate_journal(journal: &Journal) -> Result<(), ApplyError> {
    if journal.version != JOURNAL_VERSION
        || !journal.repository_root.is_absolute()
        || journal.entries.is_empty()
        || journal.entries.len() > crate::limits::MAX_FILES
    {
        return Err(ApplyError::RecoveryRequired);
    }
    let mut unique = BTreeSet::new();
    for entry in &journal.entries {
        crate::SafePath::parse(&entry.relative).map_err(|_| ApplyError::RecoveryRequired)?;
        if !unique.insert(&entry.relative)
            || !is_transaction_filename(&entry.temporary, ".new")
            || !is_transaction_filename(&entry.backup, ".bak")
        {
            return Err(ApplyError::RecoveryRequired);
        }
    }
    Ok(())
}

fn is_transaction_filename(value: &str, suffix: &str) -> bool {
    value.starts_with(".premonition-")
        && value.ends_with(suffix)
        && !value.contains('/')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn write_journal(transaction: &Path, journal: &Journal) -> Result<(), ApplyError> {
    let encoded = serde_json::to_vec(journal).map_err(|_| ApplyError::State)?;
    if u64::try_from(encoded.len()).map_err(|_| ApplyError::State)? > MAX_JOURNAL_BYTES {
        return Err(ApplyError::State);
    }
    let temporary = transaction.join("journal.new");
    let final_path = transaction.join("journal.json");
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(|_| ApplyError::State)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| ApplyError::State)?;
    file.write_all(&encoded).map_err(|_| ApplyError::State)?;
    file.sync_all().map_err(|_| ApplyError::State)?;
    fs::rename(temporary, final_path).map_err(|_| ApplyError::State)?;
    sync_directory(transaction)
}

fn read_journal(path: &Path) -> Result<Journal, ApplyError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ApplyError::RecoveryRequired)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_JOURNAL_BYTES
    {
        return Err(ApplyError::RecoveryRequired);
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| ApplyError::RecoveryRequired)?
        .take(MAX_JOURNAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ApplyError::RecoveryRequired)?;
    if u64::try_from(bytes.len()).map_err(|_| ApplyError::RecoveryRequired)? > MAX_JOURNAL_BYTES {
        return Err(ApplyError::RecoveryRequired);
    }
    serde_json::from_slice(&bytes).map_err(|_| ApplyError::RecoveryRequired)
}

fn cleanup_transaction(transaction: &Path, journal: &Journal) -> Result<(), ApplyError> {
    for entry in &journal.entries {
        let target = journal.repository_root.join(&entry.relative);
        let parent = target.parent().ok_or(ApplyError::State)?;
        for name in [&entry.temporary, &entry.backup] {
            let path = parent.join(name);
            if path.exists() {
                fs::remove_file(path).map_err(|_| ApplyError::State)?;
            }
        }
    }
    remove_transaction(transaction)
}

fn remove_transaction(transaction: &Path) -> Result<(), ApplyError> {
    fs::remove_dir_all(transaction).map_err(|_| ApplyError::State)
}

fn hash_file(path: &Path) -> Result<String, ApplyError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ApplyError::State)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_RESULT_FILE_BYTES
    {
        return Err(ApplyError::State);
    }
    let mut file = File::open(path).map_err(|_| ApplyError::State)?;
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = file.read(&mut buffer).map_err(|_| ApplyError::State)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(count).map_err(|_| ApplyError::State)?)
            .ok_or(ApplyError::State)?;
        if total > MAX_RESULT_FILE_BYTES {
            return Err(ApplyError::State);
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn validate_transaction_id(value: &str) -> Result<(), ApplyError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(ApplyError::Identifier);
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), ApplyError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| ApplyError::State)
}

fn unsafe_effective_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    fs::metadata("/proc/self").map_or(u32::MAX, |metadata| metadata.uid())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn startup_recovery_rolls_publishing_transaction_back_to_preimage() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let repository = temporary.path().join("repo");
        let state = temporary.path().join("state");
        fs::create_dir(&repository).expect("repository");
        fs::write(repository.join("file.txt"), "post\n").expect("postimage");
        fs::create_dir(&state).expect("state");
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).expect("private state");
        let transaction = state.join("tx-crash");
        fs::create_dir(&transaction).expect("transaction");
        fs::write(repository.join(".premonition-crash-0.bak"), "pre\n").expect("backup");

        let journal = Journal {
            version: JOURNAL_VERSION,
            repository_root: repository.clone(),
            phase: Phase::Publishing,
            entries: vec![JournalEntry {
                relative: "file.txt".into(),
                kind: JournalKind::Modify,
                temporary: ".premonition-crash-0.new".into(),
                backup: ".premonition-crash-0.bak".into(),
                pre_hash: Some(
                    hash_file(&repository.join(".premonition-crash-0.bak")).expect("hash"),
                ),
                post_hash: Some(hash_file(&repository.join("file.txt")).expect("hash")),
                published: true,
            }],
        };
        fs::create_dir(transaction.join("stage")).expect("stage");
        write_journal(&transaction, &journal).expect("journal");

        let engine = ApplyEngine::new(&state).expect("engine");
        assert_eq!(engine.recover().expect("recover").recovered, 1);
        assert_eq!(
            fs::read_to_string(repository.join("file.txt")).expect("preimage"),
            "pre\n"
        );
        assert!(!transaction.exists());
    }
}
