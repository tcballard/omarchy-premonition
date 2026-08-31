# DEVLOG

Append-only engineering decisions, investigations, and candid limitations.

## 2026-08-30 — S0 decisions

- A same-UID daemon is necessary for in-memory proposals, global single-flight,
  cancellation, expiry, and idempotent Apply across CLI calls.
- Quattro validates a panel+overlay manifest but loads only the panel, so v0.1
  uses one combined panel root.
- The original implementation's unbounded stdout, direct working-tree Apply,
  weak staleness token, leading-only `.git` rejection, and child-PID-only
  cancellation are gaps to close, not behaviours to port.
- Live Codex inference and Quickshell rendering are not deterministic evidence
  in this container; live workflow evidence remains issue #7.

## 2026-08-31 — S1 publication boundary

- v0.1 accepts only regular mode-`100644` textual files with a narrow ASCII
  path grammar. This intentionally excludes executable-bit edits, renames,
  copies, submodules, binary patches, and paths needing Git quoting.
- Apply generates post-images in its private state directory, then places
  durable same-directory temporary files before publication. A journal phase
  and content hashes make startup recovery deterministic without retaining the
  patch body in routine logs.
- Multi-file publication cannot be one filesystem atomic operation. The stated
  guarantee is therefore: no successful mixed result, and recovery converges
  to all-pre or all-post. A process kill can expose a transient mixed worktree
  until the daemon's mandatory startup recovery completes.

## 2026-08-31 — S2 executor decisions

- The extension boundary is an `AgentExecutor` trait, but v0.1 implements and
  claims only Codex CLI. Claude Code, OpenCode, Pi, and Wayfinder remain future
  adapters with no compatibility claim.
- Observed text is placed inside an explicitly inert delimiter in a fixed
  prompt. This reduces prompt-confusion risk; it does not make model output
  trusted. The strict parser and safety core remain the enforcement boundary.
- A live Codex inference was not used as deterministic test evidence in this
  environment. Process semantics are exercised with the compiled fixture; the
  installed Codex executable is probed for version/provenance at daemon start.

## 2026-08-31 — service ownership decisions

- The daemon owns the only active proposal and accepts one framed request per
  owner-credentialed Unix connection. This keeps QML and shell commands outside
  repository/Git/agent authority.
- Status and health never include source, rationale, patch, or repository paths.
  Proposal and copy bodies require explicit operations and are not cached.
- If Apply journal recovery cannot converge at startup, the service remains in
  `recovery_required`; only content-free status and health remain available.
- Clipboard integration is deliberately one-shot in the CLI. There is no
  watcher, history subscription, or background clipboard access in v0.1.

## 2026-08-31 — S3 UI boundary

- Quattro's one on-demand loader per plugin ID chooses `panel` ahead of other
  summoned kinds. The manifest therefore exposes a bar widget and one panel;
  the panel itself owns the review overlay.
- QML starts only fixed argv-vector commands for the Rust CLI and Omarchy IPC.
  It performs no filesystem, Git, agent, patch-validation, or Apply work.
- Selected text uses an explicit one-shot primary-selection read; clipboard
  text uses an explicit one-shot clipboard read. Both share the same CLI input
  bounds and there is no history watcher.
- The current environment has neither Quickshell nor QML lint tooling. Static
  contract/hostile-text checks and the real pinned manifest validator passed;
  visual/focus/compositor acceptance remains explicitly open.

## 2026-08-31 — S4 evidence boundary

- The demo uses the repository's compiled fake executor, never a fabricated UI
  state or network model response. It defaults to stopping after proposal
  review and requires a separate `--apply` invocation to mutate its disposable
  fixture.
- Release inputs are assembled only from `git archive HEAD`; packaging refuses
  a dirty tree, records the commit/tree/toolchain/upstream pins, and extracts
  and revalidates the resulting archive.
- The current execution sandbox blocks `AF_UNIX` socket creation at the syscall
  boundary. This is distinct from Quickshell absence: state-machine tests cover
  the protocol workflow, while external socket and live compositor evidence
  remain claims to run in an ordinary Omarchy user session.

## 2026-08-31 — S2.1 deliberate escalation

The executor now says exactly what it is doing instead of inheriting an ambient
reasoning default. Low is the ordinary path; Medium is a single repair attempt
for candidate-quality failures, not a generic retry button for broken runtimes
or changed repositories.

That distinction matters because retries expand both authority and time. A
timeout, cancellation, crash, or stale tree now ends the job immediately. The
tests also force a real descendant process to prove group termination rather
than merely checking the parent error enum.

Proposal review can show which pinned tool, configured model, and effort made
the candidate, while the bar's routine polling remains free of that detail and
all content bodies. The next move is live-shell evidence, not more executor
claims.

Source: BUILDLOG `S2.1 explicit effort and provenance closure`.

## 2026-08-31 — S3.1 compositor evidence boundary

The runtime fixture deliberately uses a real Quickshell process and the actual
plugin QML rather than simulating component state in JavaScript. A deterministic
CLI records every argv vector, introduces overlapping-call detection, and emits
hostile markup-shaped text so the fixture can assert both explicit authority
and plain-text rendering behaviour.

Headless wlroots is materially stronger evidence than parsing QML: it exercises
layer-shell windows, focusable controls, process collectors, timers and the
rendering path, and produces a compositor screenshot. It still cannot establish
whether Tom finds focus order and interaction correct in his full Omarchy
desktop. That last human observation stays open and is not renamed into an
automated claim.

Source: BUILDLOG `S3.1 real Quickshell runtime contract`.

## 2026-08-31 — S4 make the pin visible to the scanner

The runtime fixture was already execution-pinned, but its clone-first shape hid
that fact from the marketplace's deliberately small deterministic scanner. The
new sequence makes the safety order mechanical: validate the literal pin, fetch
that object alone, detach at it, verify HEAD, then execute the upstream
contract. This removes review ambiguity without weakening or broadening the
runtime test.

Source: BUILDLOG `S4 marketplace pin-fetch reconciliation`.
