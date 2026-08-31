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
