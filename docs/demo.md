# Reproducible headless demonstration

Run the deterministic synthetic workflow without mutation:

    ./scripts/demo.sh

It creates a temporary Git repository containing a.txt, starts the real daemon
and CLI against the compiled deterministic agent fixture, submits an observed
error, waits for validation, proves the file hash is unchanged, and performs
the explicit proposal-show operation.

Run the complete workflow, including the explicit Apply action:

    ./scripts/demo.sh --apply

The --apply flag is the demonstration's explicit human authorization. The
script then verifies the new worktree content and that the Git index remains
unchanged. The temporary repository is removed on exit.

This is deterministic process/safety evidence, not a claim of live model
quality. The executor's actual Codex path is separately implemented and
provenance-probed. Real Quickshell rendering is covered by the CI runtime
contract; hands-on acceptance in a complete Omarchy desktop remains issue #7.

## Live Omarchy acceptance

On a current Omarchy Quattro session with the service configured and plugin
enabled, run one command:

    ./scripts/acceptance-omarchy.sh

It checks shell IPC, runs the installed official validator, captures
content-free health, summons the panel, and captures a screenshot when grim is
available. Attach the output and keyboard/focus observations to issue #7.
