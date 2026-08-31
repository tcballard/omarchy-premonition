# Threat model and safety boundary

## Protected assets

Premonition protects allowlisted working trees, unrelated local changes,
clipboard/selection text, source content, proposal bodies, Git index state, and
the user's authority to decide whether a candidate is applied.

## Trust boundaries

| Component | Trust level | Authority |
| --- | --- | --- |
| Omarchy QML | Presentation-only, unsandboxed by Omarchy | Fixed CLI and shell-IPC argv only |
| premonition CLI | Local transport and explicit clipboard handoff | Owner-only socket; no repository logic |
| premonitiond | Trusted policy owner | One in-memory proposal, single flight, recovery |
| Codex CLI/model output | Untrusted | Read-only investigation; bounded structured output |
| Git and filesystem | Enforcement inputs | Canonical executable, roots, paths, snapshots |
| Candidate patch | Untrusted data | Strict parse/check/revalidation before review and Apply |

The shell can execute third-party QML without a sandbox, which is why QML does
not receive repository paths, source, Git commands, executor prompts, or Apply
implementation. It receives content-free status routinely and a patch body only
after explicit Review.

## Enforced invariants

- A repository is addressable only by a configured safe ID.
- Root, Git directory, Git executable, target paths, and path ancestors are
  canonicalized and revalidated.
- Traversal, symlinks, hardlinks, special files, nested .git boundaries,
  quoted/ambiguous paths, binary patches, renames, copies, and mode changes fail
  closed in v0.1.
- Generation captures HEAD, branch identity, index bytes, and bounded
  tracked/untracked worktree metadata and content.
- Investigation uses one process group, read-only Codex sandboxing, no
  approvals, strict output schema, time/size ceilings, and cancellation.
- Validation and git apply --check do not modify the repository.
- Apply rechecks identity, operation state, complete generation snapshot,
  paths, bounds, and applicability.
- Apply never runs hooks, tests, formatters, package managers, staging, commits,
  pushes, or pull-request operations.
- Status, health, recent history, errors, and routine diagnostics contain no
  source, clipboard, rationale, patch, or repository paths.

## Apply and crash semantics

Post-images are generated outside the repository. Apply then creates
same-directory temporary files and a durable content-free journal before
publishing per-file atomic renames. No command reports success for a mixed
tree. Startup recovery converges an interrupted transaction to all pre-images
or all post-images.

POSIX cannot atomically rename multiple files in different directories as one
instantaneous operation. A hard kill can therefore expose a transient mixed
worktree until mandatory recovery completes. This is explicitly not described
as instantaneous multi-file atomicity.

## Residual risks and non-goals

- A malicious root, kernel, or already-compromised same-UID account is outside
  the boundary. Such an actor can replace user files or observe user memory.
- Model reasoning can be wrong. Deterministic validation establishes safety and
  applicability, not semantic correctness; review remains essential.
- Read-only investigation still discloses allowlisted repository content to the
  locally configured Codex runtime under that runtime's account/network policy.
- Denial of service through a very large repository is handled by bounds and a
  fail-closed error, not by completing the investigation.
- v0.1 has no direct vendor API adapter, MCP, remote telemetry, clipboard
  watcher, autonomous tests, autonomous Apply, release, or marketplace action.
