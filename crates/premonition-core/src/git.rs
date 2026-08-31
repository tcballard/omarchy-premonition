//! Bounded invocation of one canonical Git executable without a shell or
//! ambient user/system configuration.

use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::limits::MAX_GIT_OUTPUT_BYTES;

const GIT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_GIT_BINARY_BYTES: u64 = 128 * 1024 * 1024;

/// Immutable identity and invocation boundary for Git.
#[derive(Clone, Eq, PartialEq)]
pub struct GitBinary {
    path: PathBuf,
    digest: [u8; 32],
    device: u64,
    inode: u64,
}

impl GitBinary {
    /// Resolves and records an absolute regular executable.
    ///
    /// # Errors
    ///
    /// Returns a stable [`GitError`] when the configured path is relative,
    /// missing, non-regular, oversized, or cannot be read.
    pub fn resolve(path: &Path) -> Result<Self, GitError> {
        use std::os::unix::fs::MetadataExt;

        if !path.is_absolute() {
            return Err(GitError::UnsafeBinary);
        }
        let canonical = fs::canonicalize(path).map_err(|_| GitError::UnsafeBinary)?;
        let metadata = fs::metadata(&canonical).map_err(|_| GitError::UnsafeBinary)?;
        if !metadata.is_file() || metadata.len() > MAX_GIT_BINARY_BYTES {
            return Err(GitError::UnsafeBinary);
        }
        let digest = hash_file(&canonical, MAX_GIT_BINARY_BYTES)?;
        Ok(Self {
            path: canonical,
            digest,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    /// Revalidates the executable immediately before use.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::BinaryChanged`] if path, type, identity, or digest
    /// differs from the resolved binary.
    pub fn revalidate(&self) -> Result<(), GitError> {
        use std::os::unix::fs::MetadataExt;

        let canonical = fs::canonicalize(&self.path).map_err(|_| GitError::BinaryChanged)?;
        let metadata = fs::metadata(&canonical).map_err(|_| GitError::BinaryChanged)?;
        if canonical != self.path
            || !metadata.is_file()
            || metadata.dev() != self.device
            || metadata.ino() != self.inode
            || hash_file(&canonical, MAX_GIT_BINARY_BYTES)? != self.digest
        {
            return Err(GitError::BinaryChanged);
        }
        Ok(())
    }

    /// Runs a bounded, configuration-neutral Git command.
    ///
    /// # Errors
    ///
    /// Returns a stable error for executable changes, spawn/I/O failures,
    /// timeout, output overflow, or a non-zero exit.
    pub fn run(
        &self,
        repository: &Path,
        arguments: &[&str],
        stdin: Option<&[u8]>,
    ) -> Result<Vec<u8>, GitError> {
        self.revalidate()?;
        let mut command = Command::new(&self.path);
        command
            .current_dir(repository)
            .env_clear()
            .env("HOME", "/nonexistent")
            .env("XDG_CONFIG_HOME", "/nonexistent")
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("TERM", "dumb")
            .env("NO_COLOR", "1")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .arg("--no-optional-locks")
            .args(["-c", "core.fsmonitor=false"])
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(["-c", "diff.external="])
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|_| GitError::Spawn)?;
        let child_stdin = child.stdin.take().ok_or(GitError::Io)?;
        let child_stdout = child.stdout.take().ok_or(GitError::Io)?;
        let child_stderr = child.stderr.take().ok_or(GitError::Io)?;
        let input = stdin.map(<[u8]>::to_vec).unwrap_or_default();

        let writer = thread::spawn(move || -> Result<(), GitError> {
            let mut stream = child_stdin;
            if !input.is_empty() {
                stream.write_all(&input).map_err(|_| GitError::Io)?;
            }
            Ok(())
        });

        let (event_tx, event_rx) = mpsc::channel();
        let stdout_reader = spawn_reader(child_stdout, MAX_GIT_OUTPUT_BYTES, event_tx.clone());
        let stderr_reader = spawn_reader(child_stderr, 64 * 1024, event_tx);
        let deadline = Instant::now() + GIT_TIMEOUT;

        let status = loop {
            if event_rx.try_recv().is_ok() {
                let _ = child.kill();
                let _ = child.wait();
                join_writer(writer)?;
                let _ = join_reader(stdout_reader);
                let _ = join_reader(stderr_reader);
                return Err(GitError::OutputLimit);
            }
            if let Some(status) = child.try_wait().map_err(|_| GitError::Io)? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                join_writer(writer)?;
                let _ = join_reader(stdout_reader);
                let _ = join_reader(stderr_reader);
                return Err(GitError::Timeout);
            }
            thread::sleep(Duration::from_millis(5));
        };

        join_writer(writer)?;
        let stdout = join_reader(stdout_reader)?;
        let stderr = join_reader(stderr_reader)?;
        if !status.success() {
            let _ = stderr;
            return Err(GitError::Exit);
        }
        Ok(stdout)
    }

    /// Returns the canonical binary path for provenance, never routine logs.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the executable SHA-256 digest.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

impl fmt::Debug for GitBinary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitBinary")
            .field("path", &"<redacted>")
            .field("device", &self.device)
            .field("inode", &self.inode)
            .finish_non_exhaustive()
    }
}

/// Stable bounded-Git errors with no captured stderr or paths.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum GitError {
    /// Configured executable is unsafe or unreadable.
    #[error("Git binary is unsafe")]
    UnsafeBinary,
    /// Resolved executable changed.
    #[error("Git binary changed")]
    BinaryChanged,
    /// Child could not be spawned.
    #[error("Git could not start")]
    Spawn,
    /// Bounded child I/O failed.
    #[error("Git I/O failed")]
    Io,
    /// Child exceeded its deadline.
    #[error("Git timed out")]
    Timeout,
    /// Child output exceeded a ceiling.
    #[error("Git output exceeded its limit")]
    OutputLimit,
    /// Child exited unsuccessfully; stderr is deliberately discarded.
    #[error("Git rejected the operation")]
    Exit,
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
    event: mpsc::Sender<()>,
) -> thread::JoinHandle<Result<Vec<u8>, GitError>> {
    thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            let count = reader.read(&mut buffer).map_err(|_| GitError::Io)?;
            if count == 0 {
                return Ok(output);
            }
            if output.len().saturating_add(count) > limit {
                let _ = event.send(());
                return Err(GitError::OutputLimit);
            }
            output.extend_from_slice(&buffer[..count]);
        }
    })
}

fn join_reader(handle: thread::JoinHandle<Result<Vec<u8>, GitError>>) -> Result<Vec<u8>, GitError> {
    handle.join().map_err(|_| GitError::Io)?
}

fn join_writer(handle: thread::JoinHandle<Result<(), GitError>>) -> Result<(), GitError> {
    handle.join().map_err(|_| GitError::Io)?
}

fn hash_file(path: &Path, limit: u64) -> Result<[u8; 32], GitError> {
    let metadata = fs::metadata(path).map_err(|_| GitError::Io)?;
    if metadata.len() > limit {
        return Err(GitError::UnsafeBinary);
    }
    let mut file = File::open(path).map_err(|_| GitError::Io)?;
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = file.read(&mut buffer).map_err(|_| GitError::Io)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(count).map_err(|_| GitError::Io)?)
            .ok_or(GitError::Io)?;
        if total > limit {
            return Err(GitError::UnsafeBinary);
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_binary_is_rejected() {
        assert_eq!(
            GitBinary::resolve(Path::new("git")),
            Err(GitError::UnsafeBinary)
        );
    }

    #[test]
    fn debug_redacts_binary_path() {
        let binary = GitBinary {
            path: PathBuf::from("/unique/secret/git"),
            digest: [0; 32],
            device: 1,
            inode: 2,
        };
        let debug = format!("{binary:?}");
        assert!(!debug.contains("/unique/secret/git"));
    }
}
