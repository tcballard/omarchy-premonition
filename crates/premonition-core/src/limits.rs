//! Fixed v0.1 safety ceilings.

/// Maximum submitted observed-error bytes.
pub const MAX_INPUT_BYTES: usize = 32 * 1024;
/// Maximum constructed executor prompt bytes.
pub const MAX_PROMPT_BYTES: usize = 96 * 1024;
/// Maximum executor stdout bytes.
pub const MAX_EXECUTOR_STDOUT_BYTES: usize = 512 * 1024;
/// Maximum executor stderr bytes.
pub const MAX_EXECUTOR_STDERR_BYTES: usize = 64 * 1024;
/// Maximum candidate patch bytes.
pub const MAX_PATCH_BYTES: usize = 256 * 1024;
/// Maximum rationale bytes.
pub const MAX_RATIONALE_BYTES: usize = 8 * 1024;
/// Maximum changed files in one candidate.
pub const MAX_FILES: usize = 8;
/// Maximum aggregate hunks in one candidate.
pub const MAX_HUNKS: usize = 128;
/// Maximum bytes in one diff line.
pub const MAX_DIFF_LINE_BYTES: usize = 16 * 1024;
/// Maximum resulting size of one touched file.
pub const MAX_RESULT_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Maximum aggregate resulting touched-file bytes.
pub const MAX_RESULT_BYTES: u64 = 8 * 1024 * 1024;
/// Maximum files in the conservative worktree snapshot.
pub const MAX_SNAPSHOT_FILES: usize = 4096;
/// Maximum aggregate regular-file bytes hashed in a snapshot.
pub const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum output accepted from any Git metadata command.
pub const MAX_GIT_OUTPUT_BYTES: usize = 512 * 1024;
/// Maximum allowlist configuration bytes.
pub const MAX_CONFIG_BYTES: u64 = 64 * 1024;
/// Maximum repositories in one allowlist.
pub const MAX_REPOSITORIES: usize = 64;
