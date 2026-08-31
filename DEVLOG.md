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
