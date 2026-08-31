#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
project_root=$(CDPATH='' cd -- "$script_dir/.." && pwd -P)
omarchy_sha=981274b20af8e85c09845071ac33c6230909f119
work=$(mktemp -d /tmp/premonition-qml.XXXXXX)
qs_pid=

cleanup() {
  if [[ -n $qs_pid ]]; then kill "$qs_pid" 2>/dev/null || true; wait "$qs_pid" 2>/dev/null || true; fi
  if [[ -d $work && $work == /tmp/premonition-qml.* ]]; then rm -rf -- "$work"; fi
}
trap cleanup EXIT INT TERM

if [[ -n ${OMARCHY_PATH-} ]]; then
  upstream=$(CDPATH='' cd -- "$OMARCHY_PATH" && pwd -P)
else
  upstream=$work/omarchy
  git clone --filter=blob:none --no-checkout https://github.com/omacom/omarchy.git "$upstream"
  git -C "$upstream" checkout --detach "$omarchy_sha"
fi
[[ $(git -C "$upstream" rev-parse HEAD) == "$omarchy_sha" ]]
[[ -z $(git -C "$upstream" status --porcelain=v1) ]]

plugin=$upstream/shell/plugins/panels/premonition
mkdir -p "$plugin/omarchy-plugin"
install -m 0644 "$project_root/manifest.json" "$plugin/manifest.json"
install -m 0644 "$project_root/omarchy-plugin/BarWidget.qml" "$plugin/omarchy-plugin/BarWidget.qml"
install -m 0644 "$project_root/omarchy-plugin/PremonitionSurface.qml" "$plugin/omarchy-plugin/PremonitionSurface.qml"
OMARCHY_PATH="$upstream" "$upstream/test/shell.d/bar-widget-contract-test.sh"

config=$work/config
home=$work/home
runtime=$work/runtime
mkdir -m 0700 -- "$config" "$home" "$runtime"
install -m 0644 "$project_root/tests/qml-runtime/shell.qml" "$config/shell.qml"
ln -s "$upstream/shell/Commons" "$config/Commons"
fake=$project_root/tests/qml-runtime/fake-premonition
result=$work/result.json
fake_log=$work/fake.log
fake_lock=$work/fake-lock
: >"$fake_log"

OMARCHY_PATH="$upstream" \
PREMONITION_BIN="$fake" \
PREMONITION_SURFACE_URL="$(realpath "$plugin/omarchy-plugin/PremonitionSurface.qml")" \
PREMONITION_QML_RESULT="$result" \
PREMONITION_FAKE_LOG="$fake_log" \
PREMONITION_FAKE_LOCK="$fake_lock" \
PREMONITION_RUNTIME_SCREENSHOT="${PREMONITION_RUNTIME_SCREENSHOT:-$work/premonition-runtime.png}" \
QML_XHR_ALLOW_FILE_READ=1 \
HOME="$home" XDG_CONFIG_HOME="$home/.config" XDG_CACHE_HOME="$home/.cache" \
XDG_STATE_HOME="$home/.local/state" XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" \
QML2_IMPORT_PATH="$upstream/shell${QML2_IMPORT_PATH:+:$QML2_IMPORT_PATH}" \
QML_IMPORT_PATH="$upstream/shell${QML_IMPORT_PATH:+:$QML_IMPORT_PATH}" \
PATH="$upstream/bin:$PATH" \
  quickshell -p "$config" --no-color >"$work/quickshell.log" 2>&1 &
qs_pid=$!

for _ in {1..120}; do
  [[ -s $result ]] && break
  if ! kill -0 "$qs_pid" 2>/dev/null; then
    sed -n '1,220p' "$work/quickshell.log" >&2
    exit 1
  fi
  sleep 0.1
done
[[ -s $result ]] || { sed -n '1,220p' "$work/quickshell.log" >&2; exit 1; }
jq -e '.ok == true' "$result" >/dev/null || { jq . "$result" >&2; exit 1; }

evidence=${PREMONITION_RUNTIME_SCREENSHOT:-$work/premonition-runtime.png}
[[ -s $evidence ]]
python3 - "$evidence" <<'PY'
from pathlib import Path
import sys
data = Path(sys.argv[1]).read_bytes()
assert data.startswith(b"\x89PNG\r\n\x1a\n") and len(data) > 1024
PY
printf 'Quickshell runtime contract passed; screenshot=%s\n' "$evidence"
