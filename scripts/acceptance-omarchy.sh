#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
project_root=$(CDPATH='' cd -- "$script_dir/.." && pwd -P)
omarchy_root=${OMARCHY_ROOT:-$HOME/.local/share/omarchy}
expected_omarchy_sha=981274b20af8e85c09845071ac33c6230909f119
expected_validator_sha=f7507e5042eb970e3dc918bdf6bf251c7557443892a77e71a17d7019ddde72c8
state_home=$HOME/.local/state
if [[ -v XDG_STATE_HOME && -n $XDG_STATE_HOME ]]; then state_home=$XDG_STATE_HOME; fi
evidence_dir=$state_home/premonition/acceptance
timestamp=$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p -- "$evidence_dir"

actual_omarchy_sha=$(git -C "$omarchy_root" rev-parse HEAD)
[[ $actual_omarchy_sha == "$expected_omarchy_sha" ]]
validator=$omarchy_root/bin/omarchy-plugin-validate
actual_validator_sha=$(sha256sum "$validator" | cut -d' ' -f1)
[[ $actual_validator_sha == "$expected_validator_sha" ]]
bash "$validator" "$project_root"
omarchy-shell shell ping
"$HOME/.local/bin/premonition" health --json | tee "$evidence_dir/$timestamp-health.json"
omarchy-shell shell summon io.github.tcballard.premonition '{}'
sleep 1

if command -v grim >/dev/null 2>&1; then
  grim "$evidence_dir/$timestamp-panel.png"
  printf 'Panel screenshot: %s\n' "$evidence_dir/$timestamp-panel.png"
else
  printf 'grim is unavailable; inspect the summoned panel manually.\n'
fi

printf 'Verify selection and clipboard submission, Review, Apply, Copy patch, Dismiss, Cancel, focus order, Escape, and truthful bar state.\n'
printf 'Live acceptance remains incomplete until the evidence is attached to issue #7.\n'
