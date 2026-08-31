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

Review candidate for v0.1 targeting Omarchy Quattro at exact commit
`981274b20af8e85c09845071ac33c6230909f119`. No release has been published and
the plugin has not been submitted to the marketplace. Machine-independent
acceptance is tracked in issues #1–#6; live Omarchy/Quickshell acceptance is a
separate gate in issue #7.

## What the workflow does

1. You choose an allowlisted repository and explicitly send the current
   selection, clipboard, or stdin text.
2. The daemon snapshots the repository and starts one read-only Codex CLI
   investigation with hard input/output/time bounds.
3. A strict textual unified diff is parsed, path-checked, and passed through
   `git apply --check` without changing the working tree.
4. The panel exposes the proposal and a full-screen plain-text diff review.
5. Only the Apply action revalidates identity, the complete generation
   snapshot, paths, size/file ceilings, and applicability before transactional
   publication.

## Source install for review

The installer requires one explicit initial allowlist entry and refuses to
overwrite an existing allowlist:

```bash
./scripts/install-user.sh my-project /absolute/path/to/repository "My project"
```

It builds locked release binaries, installs the thin plugin and hardened user
service under the current account, starts the service, and asks Omarchy to
rescan plugins. The plugin still lands under Omarchy's normal review/enable
boundary; enable `io.github.tcballard.premonition` in **Setup › Plugins** after
reviewing it.

To add repositories later, edit
`~/.config/premonition/repositories.toml`, using
[the example](examples/repositories.toml), then restart the user service:

```bash
systemctl --user restart premonition.service
```

## Stable command surface

Every command emits one bounded v1 JSON envelope. Status and health contain no
source or patch bodies.

```bash
premonition status --json
premonition repositories --json
printf '%s' 'the observed error' | premonition submit --repo my-project --stdin --json
premonition submit --repo my-project --selection --json
premonition submit --repo my-project --clipboard --json
premonition proposal show PROPOSAL_ID --json
premonition proposal copy PROPOSAL_ID --json
premonition proposal apply PROPOSAL_ID --json
premonition proposal dismiss PROPOSAL_ID --json
premonition cancel --json
```

Selection and clipboard reads are one-shot explicit actions. v0.1 does not
monitor clipboard history.

## Evidence

- [Pinned upstream world](docs/pins.md)
- [Architecture decision](docs/adr/0001-omarchy-first-architecture.md)
- [Threat model](docs/threat-model.md)
- [Deterministic headless demo](docs/demo.md)
- [BUILDLOG](BUILDLOG.md) and [DEVLOG](DEVLOG.md)

Run the non-mutating deterministic demo with `./scripts/demo.sh`; add the
explicit `--apply` flag to exercise the complete Apply path. Live Omarchy
acceptance has one capture command: `./scripts/acceptance-omarchy.sh`.

## Safety boundary

- No repository mutation occurs before explicit Apply.
- Only repositories in the configured allowlist are addressable.
- Status polling never contains clipboard, source, patch, or rationale bodies.
- The executor is read-only, bounded, cancellation-aware, and never receives
  approval to mutate the host.
- Premonition never runs hooks, tests, staging, commits, pushes, or PR creation.
- A valid/applicable patch is a candidate for human review, not proof of
  correctness.

Limits and supported patch forms are intentionally conservative; see the
threat model and ADR for the precise boundary and crash semantics.

## Licence

MIT. See [LICENSE](LICENSE).
