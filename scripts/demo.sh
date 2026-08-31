#!/usr/bin/env bash
set -euo pipefail

apply_requested=false
if (( $# == 1 )) && [[ $1 == --apply ]]; then
  apply_requested=true
elif (( $# != 0 )); then
  printf 'Usage: %s [--apply]\n' "$0" >&2
  exit 64
fi

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
project_root=$(CDPATH='' cd -- "$script_dir/.." && pwd -P)
demo_root=$(mktemp -d /tmp/premonition-demo.XXXXXX)
daemon_pid=

cleanup() {
  if [[ -n $daemon_pid ]]; then
    kill "$daemon_pid" 2>/dev/null || true
    wait "$daemon_pid" 2>/dev/null || true
  fi
  if [[ -n $demo_root && -d $demo_root && $demo_root == /tmp/premonition-demo.* ]]; then
    rm -rf -- "$demo_root"
  fi
}
trap cleanup EXIT INT TERM

repo=$demo_root/repository
runtime=$demo_root/runtime
state=$demo_root/state
mkdir -m 0700 -- "$repo" "$runtime" "$state"

/usr/bin/git -C "$repo" init -q
/usr/bin/git -C "$repo" config user.name "Premonition fixture"
/usr/bin/git -C "$repo" config user.email "fixture@example.invalid"
printf 'old\n' >"$repo/a.txt"
/usr/bin/git -C "$repo" add -- a.txt
/usr/bin/git -C "$repo" commit -q -m fixture

config=$demo_root/repositories.toml
{
  printf 'version = 1\n'
  printf 'git_binary = "/usr/bin/git"\n\n'
  printf '[[repositories]]\n'
  printf 'id = "demo"\n'
  printf 'label = "Synthetic demo"\n'
  printf 'path = "%s"\n' "$repo"
} >"$config"

cargo build --manifest-path "$project_root/Cargo.toml" --locked \
  -p premonition-cli -p premonition-daemon -p premonition-executor

socket=$runtime/premonition.sock
"$project_root/target/debug/premonitiond" \
  --config "$config" \
  --socket "$socket" \
  --state-dir "$state" \
  --codex "$project_root/target/debug/premonition-fake-agent" \
  --output-schema "$project_root/agent-output.schema.json" \
  --timeout-seconds 5 &
daemon_pid=$!

for _ in $(seq 1 50); do
  [[ -S $socket ]] && break
  sleep 0.1
done
[[ -S $socket ]]

export PREMONITION_SOCKET=$socket
cli=$project_root/target/debug/premonition
before=$(sha256sum "$repo/a.txt" | cut -d' ' -f1)
printf 'a.txt still contains the wrong value: old\n' \
  | "$cli" submit --repo demo --stdin --correlation-id demo-run --json

status=
for _ in $(seq 1 100); do
  status=$("$cli" status --json)
  state_name=$(printf '%s\n' "$status" | jq -r '.result.state // ""')
  [[ $state_name == ready ]] && break
  [[ $state_name == invalid || $state_name == error || $state_name == runtime_missing ]] && {
    printf '%s\n' "$status" >&2
    exit 1
  }
  sleep 0.05
done
proposal_id=$(printf '%s\n' "$status" | jq -r '.result.proposal_id // ""')
[[ $proposal_id =~ ^[A-Za-z0-9._-]{1,64}$ ]]

after_validation=$(sha256sum "$repo/a.txt" | cut -d' ' -f1)
[[ $before == "$after_validation" ]]
printf 'Repository unchanged before explicit Apply: %s\n' "$after_validation"

"$cli" proposal show "$proposal_id" --json | jq .

if [[ $apply_requested != true ]]; then
  printf 'Stopped before mutation. Re-run with --apply for the complete explicit-Apply demo.\n'
  exit 0
fi

"$cli" proposal apply "$proposal_id" --json
[[ $(<"$repo/a.txt") == new ]]
[[ -z $(/usr/bin/git -C "$repo" diff --cached --name-only) ]]
printf 'Explicit Apply produced the unstaged worktree change:\n'
/usr/bin/git -C "$repo" diff -- a.txt
