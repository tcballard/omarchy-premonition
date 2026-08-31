//! Deliberately narrow unified-diff parser used by validation, review, and
//! Apply. Unsupported Git patch features fail closed.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use thiserror::Error;

use crate::limits::{MAX_DIFF_LINE_BYTES, MAX_FILES, MAX_HUNKS, MAX_PATCH_BYTES};
use crate::sensitive::SensitiveText;

/// A validated repository-relative path with a narrow v0.1 grammar.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SafePath(String);

impl SafePath {
    /// Parses a non-empty slash-separated ASCII path.
    ///
    /// # Errors
    ///
    /// Returns [`DiffError::UnsafePath`] for absolute, ambiguous, `.git`,
    /// traversal, quoted, escaped, control-bearing, or oversized paths.
    pub fn parse(value: &str) -> Result<Self, DiffError> {
        if value.is_empty()
            || value.len() > 240
            || value.starts_with('/')
            || value.ends_with('/')
            || value.contains('\0')
            || value.contains('\\')
            || value.contains("//")
            || value.starts_with('"')
        {
            return Err(DiffError::UnsafePath);
        }
        for component in value.split('/') {
            if component.is_empty()
                || component.len() > 120
                || matches!(component, "." | "..")
                || component.eq_ignore_ascii_case(".git")
                || !component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            {
                return Err(DiffError::UnsafePath);
            }
        }
        Ok(Self(value.into()))
    }

    /// Returns the repository-relative path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    /// Returns the repository-relative UTF-8 path for Git/staging operations.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SafePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SafePath(<redacted>)")
    }
}

/// Supported textual file changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    /// Existing regular file becomes a new regular-file image.
    Modify,
    /// A new regular file is created beneath an existing safe parent.
    Add,
    /// An existing regular file is removed.
    Delete,
}

/// One parsed file section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileChange {
    /// Validated repository-relative path.
    pub path: SafePath,
    /// Supported change kind.
    pub kind: ChangeKind,
    /// Number of validated hunks in this section.
    pub hunks: usize,
}

/// One bounded, strictly parsed candidate patch.
#[derive(Clone, Eq, PartialEq)]
pub struct ParsedPatch {
    raw: SensitiveText,
    files: Vec<FileChange>,
    hunks: usize,
}

impl ParsedPatch {
    /// Parses a complete patch with no leading/trailing prose.
    ///
    /// # Errors
    ///
    /// Returns a stable [`DiffError`] for malformed, unsafe, oversized, or
    /// unsupported patch syntax.
    #[allow(clippy::too_many_lines)] // Keeping the state machine contiguous makes fail-closed review easier.
    pub fn parse(raw: String) -> Result<Self, DiffError> {
        if raw.is_empty() || raw.len() > MAX_PATCH_BYTES {
            return Err(DiffError::PatchSize);
        }
        if !raw.ends_with('\n') || raw.contains('\0') || raw.contains('\r') {
            return Err(DiffError::Malformed);
        }
        let lines: Vec<&str> = raw.split_inclusive('\n').collect();
        if lines.iter().any(|line| line.len() > MAX_DIFF_LINE_BYTES) {
            return Err(DiffError::LineSize);
        }

        let mut index = 0;
        let mut files = Vec::new();
        let mut seen = BTreeSet::new();
        let mut total_hunks = 0;

        while index < lines.len() {
            let path = parse_diff_header(lines[index])?;
            if !seen.insert(path.clone()) {
                return Err(DiffError::DuplicatePath);
            }
            index += 1;

            let mut declared_add = false;
            let mut declared_delete = false;
            let mut saw_index = false;
            while index < lines.len() && !lines[index].starts_with("--- ") {
                let metadata = lines[index].trim_end_matches('\n');
                if metadata == "new file mode 100644" && !declared_add && !declared_delete {
                    declared_add = true;
                } else if metadata == "deleted file mode 100644"
                    && !declared_add
                    && !declared_delete
                {
                    declared_delete = true;
                } else if metadata.starts_with("index ") && !saw_index {
                    validate_index_line(metadata)?;
                    saw_index = true;
                } else {
                    return Err(DiffError::UnsupportedFeature);
                }
                index += 1;
            }
            if index + 1 >= lines.len() {
                return Err(DiffError::Malformed);
            }

            let old = lines[index]
                .strip_prefix("--- ")
                .and_then(|line| line.strip_suffix('\n'))
                .ok_or(DiffError::Malformed)?;
            let new = lines[index + 1]
                .strip_prefix("+++ ")
                .and_then(|line| line.strip_suffix('\n'))
                .ok_or(DiffError::Malformed)?;
            index += 2;

            let expected_old = format!("a/{}", path.as_str());
            let expected_new = format!("b/{}", path.as_str());
            let kind = match (old, new) {
                ("/dev/null", value) if value == expected_new && declared_add => ChangeKind::Add,
                (value, "/dev/null") if value == expected_old && declared_delete => {
                    ChangeKind::Delete
                }
                (old_value, new_value)
                    if old_value == expected_old
                        && new_value == expected_new
                        && !declared_add
                        && !declared_delete =>
                {
                    ChangeKind::Modify
                }
                _ => return Err(DiffError::HeaderMismatch),
            };

            let mut file_hunks = 0;
            while index < lines.len() && !lines[index].starts_with("diff --git ") {
                if !lines[index].starts_with("@@ ") {
                    return Err(DiffError::Malformed);
                }
                let (old_count, new_count) = parse_hunk_header(lines[index])?;
                index += 1;
                let mut old_seen = 0_u64;
                let mut new_seen = 0_u64;
                while index < lines.len()
                    && !lines[index].starts_with("@@ ")
                    && !lines[index].starts_with("diff --git ")
                {
                    let line = lines[index];
                    match line.as_bytes().first().copied() {
                        Some(b' ') => {
                            old_seen = old_seen.checked_add(1).ok_or(DiffError::Malformed)?;
                            new_seen = new_seen.checked_add(1).ok_or(DiffError::Malformed)?;
                        }
                        Some(b'-') => {
                            old_seen = old_seen.checked_add(1).ok_or(DiffError::Malformed)?;
                        }
                        Some(b'+') => {
                            new_seen = new_seen.checked_add(1).ok_or(DiffError::Malformed)?;
                        }
                        Some(b'\\') if line == "\\ No newline at end of file\n" => {}
                        _ => return Err(DiffError::Malformed),
                    }
                    index += 1;
                }
                if old_seen != old_count || new_seen != new_count {
                    return Err(DiffError::HunkCount);
                }
                file_hunks += 1;
                total_hunks += 1;
                if total_hunks > MAX_HUNKS {
                    return Err(DiffError::TooManyHunks);
                }
            }
            if file_hunks == 0 {
                return Err(DiffError::NoHunks);
            }
            files.push(FileChange {
                path,
                kind,
                hunks: file_hunks,
            });
            if files.len() > MAX_FILES {
                return Err(DiffError::TooManyFiles);
            }
        }

        if files.is_empty() {
            return Err(DiffError::NoFiles);
        }
        Ok(Self {
            raw: SensitiveText::new(raw),
            files,
            hunks: total_hunks,
        })
    }

    /// Returns parsed file plans without exposing patch contents.
    #[must_use]
    pub fn files(&self) -> &[FileChange] {
        &self.files
    }

    /// Returns the validated patch body for an explicit Git/review operation.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.raw.expose()
    }

    /// Returns the patch byte count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.raw.len()
    }

    /// Parsed patches are structurally guaranteed to contain at least one file.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// Returns the total validated hunk count.
    #[must_use]
    pub fn hunk_count(&self) -> usize {
        self.hunks
    }
}

impl fmt::Debug for ParsedPatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParsedPatch")
            .field("bytes", &self.len())
            .field("files", &self.files.len())
            .field("hunks", &self.hunks)
            .finish_non_exhaustive()
    }
}

/// Strict parsing failures without content-bearing context.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DiffError {
    /// Patch is empty or oversized.
    #[error("patch size is invalid")]
    PatchSize,
    /// One line exceeds its ceiling.
    #[error("diff line is too large")]
    LineSize,
    /// General structural failure.
    #[error("patch is malformed")]
    Malformed,
    /// Path violates the narrow grammar.
    #[error("patch path is unsafe")]
    UnsafePath,
    /// Headers disagree about the touched path or change kind.
    #[error("patch headers disagree")]
    HeaderMismatch,
    /// Unsupported Git patch feature was present.
    #[error("patch feature is unsupported")]
    UnsupportedFeature,
    /// Hunk body counts do not match the header.
    #[error("hunk counts disagree")]
    HunkCount,
    /// File section has no hunks.
    #[error("file section has no hunks")]
    NoHunks,
    /// Patch has no file sections.
    #[error("patch has no files")]
    NoFiles,
    /// Duplicate file section.
    #[error("patch repeats a path")]
    DuplicatePath,
    /// Changed-file ceiling exceeded.
    #[error("patch changes too many files")]
    TooManyFiles,
    /// Hunk ceiling exceeded.
    #[error("patch has too many hunks")]
    TooManyHunks,
}

fn parse_diff_header(line: &str) -> Result<SafePath, DiffError> {
    let header = line.strip_suffix('\n').ok_or(DiffError::Malformed)?;
    let fields: Vec<&str> = header.split(' ').collect();
    if fields.len() != 4 || fields[0] != "diff" || fields[1] != "--git" {
        return Err(DiffError::Malformed);
    }
    let old = fields[2].strip_prefix("a/").ok_or(DiffError::UnsafePath)?;
    let new = fields[3].strip_prefix("b/").ok_or(DiffError::UnsafePath)?;
    if old != new {
        return Err(DiffError::HeaderMismatch);
    }
    SafePath::parse(old)
}

fn validate_index_line(line: &str) -> Result<(), DiffError> {
    let fields: Vec<&str> = line.split(' ').collect();
    if !(fields.len() == 2 || (fields.len() == 3 && fields[2] == "100644")) {
        return Err(DiffError::UnsupportedFeature);
    }
    let (old, new) = fields[1].split_once("..").ok_or(DiffError::Malformed)?;
    if old.len() < 7
        || new.len() < 7
        || !old.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !new.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DiffError::Malformed);
    }
    Ok(())
}

fn parse_hunk_header(line: &str) -> Result<(u64, u64), DiffError> {
    let header = line.strip_suffix('\n').ok_or(DiffError::Malformed)?;
    let fields: Vec<&str> = header.split(' ').collect();
    if fields.len() < 4 || fields[0] != "@@" || fields[3] != "@@" {
        return Err(DiffError::Malformed);
    }
    let old = fields[1].strip_prefix('-').ok_or(DiffError::Malformed)?;
    let new = fields[2].strip_prefix('+').ok_or(DiffError::Malformed)?;
    Ok((parse_range(old)?, parse_range(new)?))
}

fn parse_range(value: &str) -> Result<u64, DiffError> {
    let (start, count) = value.split_once(',').unwrap_or((value, "1"));
    if start.is_empty()
        || count.is_empty()
        || !start.bytes().all(|byte| byte.is_ascii_digit())
        || !count.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(DiffError::Malformed);
    }
    let _: u64 = start.parse().map_err(|_| DiffError::Malformed)?;
    count.parse().map_err(|_| DiffError::Malformed)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    const MODIFY: &str = "diff --git a/src/main.rs b/src/main.rs\nindex 7898192..422c2b7 100644\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\n";
    const ADD: &str = "diff --git a/new.txt b/new.txt\nnew file mode 100644\nindex 0000000..3e75765\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hello\n";
    const DELETE: &str = "diff --git a/old.txt b/old.txt\ndeleted file mode 100644\nindex ce01362..0000000\n--- a/old.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";

    #[test]
    fn parses_supported_modify_add_and_delete() {
        for (raw, kind) in [
            (MODIFY, ChangeKind::Modify),
            (ADD, ChangeKind::Add),
            (DELETE, ChangeKind::Delete),
        ] {
            let parsed = ParsedPatch::parse(raw.into()).expect("fixture parses");
            assert_eq!(parsed.files().len(), 1);
            assert_eq!(parsed.files()[0].kind, kind);
            assert_eq!(parsed.hunk_count(), 1);
        }
    }

    #[test]
    fn rejects_prose_fences_binary_rename_and_mode_changes() {
        for raw in [
            "Here is the patch:\n",
            "```diff\n",
            "diff --git a/a b/a\nGIT binary patch\n",
            "diff --git a/a b/a\nsimilarity index 100%\nrename from a\nrename to b\n",
            "diff --git a/a b/a\nold mode 100644\nnew mode 100755\n",
        ] {
            assert!(ParsedPatch::parse(raw.into()).is_err());
        }
    }

    #[test]
    fn rejects_traversal_git_components_spaces_and_quotes() {
        for path in ["../x", "src/.git/config", "a//b", "has space", "\"quoted\""] {
            assert_eq!(SafePath::parse(path), Err(DiffError::UnsafePath));
        }
    }

    #[test]
    fn rejects_header_disagreement_and_duplicate_sections() {
        let mismatch = MODIFY.replace("b/src/main.rs", "b/src/other.rs");
        assert_eq!(ParsedPatch::parse(mismatch), Err(DiffError::HeaderMismatch));
        let duplicate = format!("{MODIFY}{MODIFY}");
        assert_eq!(ParsedPatch::parse(duplicate), Err(DiffError::DuplicatePath));
    }

    #[test]
    fn rejects_incorrect_hunk_counts_and_missing_newline() {
        let bad_count = MODIFY.replace("@@ -1 +1 @@", "@@ -1,2 +1 @@");
        assert_eq!(ParsedPatch::parse(bad_count), Err(DiffError::HunkCount));
        assert_eq!(
            ParsedPatch::parse(MODIFY.trim_end().into()),
            Err(DiffError::Malformed)
        );
    }

    #[test]
    fn parsed_debug_redacts_paths_and_patch_body() {
        let parsed = ParsedPatch::parse(MODIFY.into()).expect("fixture parses");
        let debug = format!("{parsed:?}");
        assert!(!debug.contains("src/main.rs"));
        assert!(!debug.contains("old"));
    }
}
