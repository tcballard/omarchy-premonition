# ADR 0001: Omarchy-first process and plugin architecture

- Status: Accepted
- Date: 2026-08-30
- Issues: #1, #2, #3, #4

## Context

Omarchy loads third-party QML as unsandboxed code inside one long-lived
Quickshell process. Premonition handles sensitive clipboard text, source,
agent output, and a candidate patch. It must enforce a global single-flight
executor, keep proposal bodies out of persistent status, survive separate CLI
calls, and make Apply the only repository mutation boundary.

## Decision

Use a Rust workspace with these ownership boundaries:

- `premonition-protocol`: versioned, bounded wire DTOs and stable errors.
- `premonition-core`: allowlists, repository identity, strict diff parsing,
  validation, staleness, Apply transaction, and recovery.
- `premonition-executor`: one audited Codex CLI adapter plus a mockable trait,
  streaming bounds, process-group cancellation, and redacted provenance.
- `premonitiond`: same-UID user service and sole owner of the active executor,
  in-memory proposals, bounded history, TTL, idempotency, and recovery.
- `premonition`: thin JSON CLI client over an owner-only Unix socket.
- `omarchy-plugin/`: presentation only; it polls content-free status and fetches
  proposal bodies only for explicit review.

The root manifest uses `schemaVersion: 1`, `bar-widget` and one `panel` entry.
The panel owns both the compact view and an internal full-screen review window.

The daemon socket lives at `$XDG_RUNTIME_DIR/premonition/v1.sock` in a 0700
directory with mode 0600 and same-UID peer checks. No `/tmp` fallback exists.

Codex is the first executor. It runs no-approval, read-only, ephemeral,
strict-config JSONL with a schema-constrained response; prompts use stdin.

## Apply guarantee

Identity, staleness, paths, bounds, and `git apply --check` are revalidated
before publication. Publication uses staged same-filesystem files, a durable
content-free journal, per-file atomic rename, and recovery before new commands.

POSIX does not provide one instantaneous transaction across arbitrary files in
multiple directories. No command may report success for a mixed tree; after
completion or mandatory recovery the tree is entirely the recorded pre-image
or post-image. A crash can expose a transient mixed tree until recovery. The
product must not call this instantaneous multi-file atomicity.

## Consequences

- The user service is part of v0.1, not an optional optimisation.
- Proposal bodies disappear on daemon shutdown by design.
- Status and routine diagnostics are content-free.
- Live Omarchy rendering remains a separate acceptance gate.
