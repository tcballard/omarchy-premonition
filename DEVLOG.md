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
