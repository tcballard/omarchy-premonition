# Omarchy Premonition

Omarchy Premonition is a review-first bridge from an observed error to a
bounded candidate patch. It investigates an explicitly selected, allowlisted
repository; validates a textual unified diff without changing the repository;
and mutates files only after an explicit **Apply** action.

The v0.1 architecture keeps QML deliberately thin. A same-UID Rust user
service owns repository access, the single-flight Codex executor, in-memory
proposal bodies, cancellation, validation, and crash recovery. A versioned
JSON CLI is the only interface used by the Omarchy bar widget and panel.

## Status

Active development for v0.1. No release has been published. Machine-independent
acceptance is tracked in issues #1–#6; live Omarchy/Quickshell acceptance is a
separate gate in issue #7.

## Safety boundary

- No repository mutation occurs before explicit Apply.
- Only repositories in the configured allowlist are addressable.
- Status polling never contains clipboard, source, patch, or rationale bodies.
- The executor is read-only, bounded, cancellation-aware, and never receives
  approval to mutate the host.
- Premonition never runs hooks, tests, staging, commits, pushes, or PR creation.
- A valid/applicable patch is a candidate for human review, not proof of
  correctness.

See [ADR 0001](docs/adr/0001-omarchy-first-architecture.md) and the
[pinned-world record](docs/pins.md) for the normative v0.1 foundation.

## Licence

MIT. See [LICENSE](LICENSE).
