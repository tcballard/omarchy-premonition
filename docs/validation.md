# Machine-independent validation evidence

Validated on 2026-08-31 with Rust 1.98.0 against Omarchy Quattro commit
`981274b20af8e85c09845071ac33c6230909f119`. The upstream validator file had
SHA-256 `f7507e5042eb970e3dc918bdf6bf251c7557443892a77e71a17d7019ddde72c8`.

| Gate | Exact command | Result |
| --- | --- | --- |
| Format | `cargo fmt --check` | Passed |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Passed |
| Tests | `cargo test --workspace --all-features` | 42 passed |
| Documentation | `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps` | Passed |
| Release build | `cargo build --workspace --all-features --release --locked` | Passed |
| Supply chain | `cargo deny check` | Advisories, bans, licences, and sources passed; one permitted duplicate-version warning |
| QML/hostile text | `python3 scripts/check-qml.py` | Passed |
| Credential patterns | `python3 scripts/check-secrets.py` | Passed |
| Shell syntax | `bash -n scripts/*.sh` | Passed |
| Plugin contract | `bash "$PINNED_OMARCHY/bin/omarchy-plugin-validate" .` | Passed, no diagnostics |
| Mechanical PR preflight | `preflight_pr.py --repo . --base main` | Passed, no blockers or warnings |

ShellCheck is not installed in this execution container. The immutable-action
CI recipe installs it from Ubuntu 24.04 and runs `shellcheck scripts/*.sh`; its
first run identified only shell portability/lint findings, which were corrected
before review handoff.

Packaging is invoked only from a clean branch with:

```bash
OMARCHY_VALIDATOR="$PINNED_OMARCHY/bin/omarchy-plugin-validate" ./scripts/package.sh
```

The script refuses a dirty tree, archives `HEAD`, builds and tests that clean
extraction, validates dependencies and the staged plugin, records the Git
commit/tree and pins in `BUILD-METADATA`, creates the bundle, verifies its
SHA-256, extracts it again, and revalidates the plugin and both executables.

The external Unix-socket demo was attempted and failed before any submission
because this sandbox denies `AF_UNIX` socket creation with `EPERM`. The daemon
emitted only `premonitiond: socket bind syscall failed`. Complete protocol and
Apply behaviour passed through the in-process state-machine suite. Live
Quickshell acceptance is intentionally not claimed and remains issue #7.
