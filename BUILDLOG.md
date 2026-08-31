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

## 2026-08-31 — S2 bounded Codex executor

- Added the first genuine agent path through the local Codex CLI using
  `read-only`, `--ask-for-approval never`, ephemeral sessions, ignored user
  config/rules, JSONL events, and a strict final-output schema.
- Canonicalized and hashed the executable and schema, revalidating both before
  each investigation. Recorded content-free version and SHA-256 provenance.
- Bounded prompt, stdout, stderr, patch, rationale, version probe, and runtime.
  Timeout/cancellation/output overflow kill the entire child process group.
- Added a compiled fake agent covering success, malformed structured output,
  hostile terminal text, crash, overflow, timeout, and cancellation.
- `cargo test -p premonition-executor`: 7 passed; strict all-target Clippy with
  `-D warnings`: passed.

## 2026-08-31 — versioned service and CLI

- Added an owner-only Unix socket with peer-UID checks, strict framed protocol,
  bounded reads/writes, private socket/state directories, and stale-socket
  handling.
- Added the same-UID in-memory state machine with global single-flight,
  cancellation, content-free status/recent history, request-ID replay checks,
  bounded terminal history, and explicit review/copy/dismiss/Apply operations.
- Body-bearing review/copy results are deliberately excluded from the request
  cache so source/patch data is released when the active proposal is removed.
- Added the JSON-only CLI with explicit stdin or one-shot clipboard submission;
  copy writes directly to a canonical clipboard process without printing the
  patch.
- Synthetic daemon tests cover single-flight, read-only pre-Apply state,
  explicit Apply, unchanged Git index, cancellation, and idempotent replay.
- strict CLI/daemon/executor all-target Clippy with `-D warnings`: passed.

## 2026-08-31 — S3 Omarchy Quattro interface

- Added the thin bar entry point with idle, working, ready, invalid, error,
  runtime-missing, applying, and recovery-required states. Polling is bounded,
  content-free, single-process, and non-overlapping.
- Added the summoned panel with explicit selection/clipboard actions, repository
  allowlist picker, bounded in-memory recent state, and Review, Apply, Copy,
  Dismiss, and Cancel controls.
- Added an internal full-screen diff-review overlay. JSON-derived strings render
  as plain text; Apply remains disabled in the compact panel until that proposal
  body has been fetched for review.
- Added a hardened systemd user unit, explicit one-repository installer, and
  example allowlist.
- QML structural/hostile-text checks: passed. Bash syntax check: passed.
- Official `omarchy-plugin-validate` from pinned Quattro
  `981274b20af8e85c09845071ac33c6230909f119` (validator SHA-256
  `f7507e5042eb970e3dc918bdf6bf251c7557443892a77e71a17d7019ddde72c8`):
  passed with exit 0 and no diagnostics.
- Live Quickshell rendering was not available and remains issue #7.

## 2026-08-31 — S4 reproducible evidence

- Added a synthetic repository and compiled fake-agent demonstration that
  proves validation is read-only, stops before mutation by default, and offers
  an independently explicit `--apply` continuation.
- Added a one-command live Omarchy acceptance capture that checks the pinned
  validator before asking the reviewer to exercise selection, clipboard,
  review, copy, dismiss, cancel, and Apply interactions.
- Added an explicit threat model, stable command documentation, dependency and
  licence policy, credential-pattern check, and pinned CI acceptance recipe.
- Extended the real Git fixture to preserve a pre-existing unrelated worktree
  edit across transactional Apply.
- This container denies Unix socket creation with `EPERM`; the daemon therefore
  reported its stable content-free bind error and the external socket demo
  could not run here. The same complete state-machine workflow passes in-process
  tests. This is recorded as an environment gate, not live acceptance.
