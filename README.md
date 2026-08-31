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

Version 0.1.0 targets Omarchy Quattro at exact commit
`981274b20af8e85c09845071ac33c6230909f119`. Machine-independent acceptance is
complete in issues #1–#6; live Omarchy/Quickshell acceptance remains a separate
hands-on desktop gate in issue #7. Real Quickshell rendering and the full
deterministic interaction contract run in CI under headless wlroots.

## What the workflow does

1. You choose an allowlisted repository and explicitly send the current
   selection, clipboard, or stdin text.
2. The daemon snapshots the repository and starts one read-only Codex CLI
   investigation with hard input/output/time bounds. It starts at Low effort
   and permits one Medium retry only after malformed or invalid candidate
   output; runtime, cancellation, timeout, and repository failures never retry.
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

### Dependencies

- Omarchy Quattro compatible with the pinned contract above, including
  Quickshell and `omarchy-shell`.
- A systemd user session, Git, and Rust 1.98.0 for the source build.
- An installed and configured Codex CLI. Premonition invokes it read-only with
  the configured `gpt-5.6-sol` model; it does not integrate directly with a
  vendor API or store provider credentials.

The plugin needs its native daemon, CLI, allowlist, and user service before it
can function. A marketplace listing therefore requires manual setup rather than
the standard clone-only installation path.

To add repositories later, edit
`~/.config/premonition/repositories.toml`, using
[the example](examples/repositories.toml), then restart the user service:

```bash
systemctl --user restart premonition.service
```

## Removal

Disable the user service, remove the installed executables and plugin files,
then ask Omarchy to rescan plugins:

```bash
systemctl --user disable --now premonition.service
rm -f ~/.config/systemd/user/premonition.service
rm -f ~/.local/bin/premonition ~/.local/bin/premonitiond
rm -rf ~/.local/share/premonition
rm -rf ~/.config/omarchy/plugins/io.github.tcballard.premonition
systemctl --user daemon-reload
omarchy-shell shell rescanPlugins
```

The repository allowlist and transaction state are deliberately preserved.
After reviewing them, remove those separately if desired:

```bash
rm -rf ~/.config/premonition ~/.local/state/premonition
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

Explicit proposal review includes bounded content-free executor evidence: the
canonical tool version and SHA-256, configured model, and the Low or Medium
effort that produced the validated candidate. Routine status polling excludes
that evidence as well as patch and source bodies.

## Evidence

- [Pinned upstream world](docs/pins.md)
- [Architecture decision](docs/adr/0001-omarchy-first-architecture.md)
- [Threat model](docs/threat-model.md)
- [Deterministic headless demo](docs/demo.md)
- [Machine-independent validation](docs/validation.md)
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
