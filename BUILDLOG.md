# BUILDLOG

Append-only evidence of build stages and verification outcomes.

## 2026-08-30T22:35:38Z — S0 world pinning

- Verified the implementation repository existed and was completely empty.
- With explicit owner authority, established remote `main` at
  `3e0657a94024ea30fd477c65c2db1117cc55e8aa` using a title-only README because
  GitHub's connected API could not publish a parentless empty commit.
- Created branch `feat/omarchy-native-v0.1` from that exact base.
- Pinned official Omarchy Quattro to
  `981274b20af8e85c09845071ac33c6230909f119`.
- Audited original Premonition at
  `2a4a85b0575d889c9e49c12e481dd0a16147d1ea` without modifying it.
- Excluded `build-omarchy-plugins` because it had no tagged release.
- Created exact implementation issues #1–#6 and live acceptance issue #7.

## 2026-08-31 — workspace recovery

- The transient local checkout was cleared before its first feature commit was
  pushed. Remote `main` and issues were unaffected.
- Reconstructed S0 from the recorded pins and ADR; subsequent validated logical
  commits are pushed immediately after creation.

## 2026-08-31 — strict protocol v1

- Added a four-byte-length-prefixed JSON v1 contract with strict externally
  tagged request operations and `deny_unknown_fields` parameter objects.
- Bounded input, frames, patch/rationale response bodies, repository lists, and
  recent status history.
- Added content-free status/error enums and explicit body-bearing show/copy
  operations.
- `cargo test -p premonition-protocol`: 11 passed.
- strict protocol Clippy with all targets and `-D warnings`: passed.

## 2026-08-31 — S1 headless safety core

- Added eager repository allowlisting with canonical root, Git-directory, and
  pinned Git-binary identity checks.
- Added conservative HEAD/index/tracked+untracked worktree snapshots and stale
  proposal rejection before and during Apply.
- Added a deliberately narrow unified-diff parser for bounded textual
  add/modify/delete changes; traversal, `.git`, symlinks, hardlinks, binary
  patches, renames, mode changes, malformed hunks, and nested repositories fail
  closed.
- Added off-repository post-image generation plus private, journalled,
  per-file atomic publication. Recovery resolves an interrupted publication to
  the complete pre-Apply or post-Apply state; Git index and hooks are untouched.
- `cargo test -p premonition-core`: 18 passed, including a real synthetic Git
  repository and interrupted-publication recovery.
- strict core Clippy with all targets and `-D warnings`: passed.
