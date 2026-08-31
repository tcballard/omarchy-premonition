#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
project_root=$(CDPATH= cd -- "$script_dir/.." && pwd -P)
validator=${OMARCHY_VALIDATOR-}
expected_validator_sha=f7507e5042eb970e3dc918bdf6bf251c7557443892a77e71a17d7019ddde72c8
omarchy_sha=981274b20af8e85c09845071ac33c6230909f119

if [[ -n $(git -C "$project_root" status --porcelain=v1 --untracked-files=all) ]]; then
  printf 'Packaging requires a clean Git tree.\n' >&2
  exit 1
fi
if [[ -z $validator || ! -f $validator ]]; then
  printf 'Set OMARCHY_VALIDATOR to the pinned upstream validator.\n' >&2
  exit 64
fi
actual_validator_sha=$(sha256sum "$validator" | cut -d' ' -f1)
if [[ $actual_validator_sha != "$expected_validator_sha" ]]; then
  printf 'Pinned validator digest mismatch.\n' >&2
  exit 1
fi

commit=$(git -C "$project_root" rev-parse HEAD)
tree=$(git -C "$project_root" rev-parse 'HEAD^{tree}')
epoch=$(git -C "$project_root" show -s --format=%ct HEAD)
work=$(mktemp -d /tmp/premonition-package.XXXXXX)
cleanup() {
  if [[ -n $work && -d $work && $work == /tmp/premonition-package.* ]]; then
    rm -rf -- "$work"
  fi
}
trap cleanup EXIT INT TERM

mkdir -p "$work/source" "$work/bundle/bin" \
  "$work/bundle/share/premonition" \
  "$work/bundle/share/omarchy/plugins/io.github.tcballard.premonition/omarchy-plugin" \
  "$work/bundle/share/systemd/user" "$work/bundle/share/doc/omarchy-premonition"
git -C "$project_root" archive --format=tar "$commit" | tar -xf - -C "$work/source"

(cd "$work/source" && cargo build --locked --release -p premonition-cli -p premonition-daemon)
(cd "$work/source" && cargo deny check)

install -m 0755 "$work/source/target/release/premonition" "$work/bundle/bin/premonition"
install -m 0755 "$work/source/target/release/premonitiond" "$work/bundle/bin/premonitiond"
install -m 0644 "$work/source/agent-output.schema.json" "$work/bundle/share/premonition/agent-output.schema.json"
plugin="$work/bundle/share/omarchy/plugins/io.github.tcballard.premonition"
install -m 0644 "$work/source/manifest.json" "$plugin/manifest.json"
install -m 0644 "$work/source/omarchy-plugin/BarWidget.qml" "$plugin/omarchy-plugin/BarWidget.qml"
install -m 0644 "$work/source/omarchy-plugin/PremonitionSurface.qml" "$plugin/omarchy-plugin/PremonitionSurface.qml"
install -m 0644 "$work/source/systemd/premonition.service" "$work/bundle/share/systemd/user/premonition.service"
install -m 0644 "$work/source/LICENSE" "$work/source/README.md" "$work/source/docs/threat-model.md" \
  "$work/bundle/share/doc/omarchy-premonition/"

cat >"$work/bundle/BUILD-METADATA" <<EOF
git_commit=$commit
git_tree=$tree
source_date_epoch=$epoch
rust_toolchain=1.98.0
omarchy_quattro_commit=$omarchy_sha
omarchy_validator_sha256=$expected_validator_sha
EOF

bash "$validator" "$plugin"
find "$work/bundle" -exec touch -h -d "@$epoch" {} +
mkdir -p "$project_root/dist"
archive="$project_root/dist/omarchy-premonition-$commit.tar.gz"
tar --sort=name --mtime="@$epoch" --owner=0 --group=0 --numeric-owner \
  -C "$work/bundle" -czf "$archive" .
sha256sum "$archive" >"$archive.sha256"

mkdir "$work/recheck"
tar -xzf "$archive" -C "$work/recheck"
test -x "$work/recheck/bin/premonition"
test -x "$work/recheck/bin/premonitiond"
grep -Fx "git_commit=$commit" "$work/recheck/BUILD-METADATA" >/dev/null
bash "$validator" "$work/recheck/share/omarchy/plugins/io.github.tcballard.premonition"
"$work/recheck/bin/premonition" --version >/dev/null
"$work/recheck/bin/premonitiond" --version >/dev/null

printf 'Built and revalidated %s\n' "$archive"
cat "$archive.sha256"
