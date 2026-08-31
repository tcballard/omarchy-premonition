#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'Usage: %s <repository-id> <repository-path> [display-label]\n' "$0" >&2
}

if (( $# < 2 || $# > 3 )); then
  usage
  exit 64
fi

repo_id=$1
repo_input=$2
repo_label=$repo_id
if (( $# == 3 )); then repo_label=$3; fi

if [[ ! $repo_id =~ ^[A-Za-z0-9._-]{1,64}$ ]]; then
  printf 'Repository ID must match [A-Za-z0-9._-] and be at most 64 bytes.\n' >&2
  exit 64
fi
label_bytes=$(printf %s "$repo_label" | wc -c)
if [[ -z $repo_label || $label_bytes -gt 80 || $repo_label == *$'\n'* ]]; then
  printf 'Display label must be one line and at most 80 bytes.\n' >&2
  exit 64
fi

repo_path=$(realpath -e -- "$repo_input")
git_binary=$(realpath -e -- "$(command -v git)")
top_level=$("$git_binary" -C "$repo_path" rev-parse --show-toplevel)
top_level=$(realpath -e -- "$top_level")
if [[ $top_level != "$repo_path" ]]; then
  printf 'Repository path must be the exact Git worktree root.\n' >&2
  exit 65
fi
for value in "$repo_path" "$git_binary" "$repo_label"; do
  if [[ $value == *'"'* || $value == *'\'* || $value == *$'\r'* ]]; then
    printf 'Installer cannot safely encode quotes, backslashes, or carriage returns.\n' >&2
    exit 65
  fi
done

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
project_root=$(CDPATH= cd -- "$script_dir/.." && pwd -P)
config_home=$HOME/.config
if [[ -v XDG_CONFIG_HOME && -n $XDG_CONFIG_HOME ]]; then config_home=$XDG_CONFIG_HOME; fi
config_dir=$config_home/premonition
data_dir=$HOME/.local/share
plugin_dir=$config_home/omarchy/plugins/io.github.tcballard.premonition
user_bin=$HOME/.local/bin
user_units=$config_home/systemd/user

if [[ -e $config_dir/repositories.toml ]]; then
  printf 'Refusing to overwrite existing allowlist: %s\n' "$config_dir/repositories.toml" >&2
  exit 73
fi

cargo build --manifest-path "$project_root/Cargo.toml" --locked --release \
  -p premonition-cli -p premonition-daemon

install -d -m 0700 -- "$config_dir"
install -d -m 0755 -- "$user_bin" "$data_dir/premonition" "$plugin_dir/omarchy-plugin" "$user_units"
install -m 0755 -- "$project_root/target/release/premonition" "$user_bin/premonition"
install -m 0755 -- "$project_root/target/release/premonitiond" "$user_bin/premonitiond"
install -m 0644 -- "$project_root/agent-output.schema.json" "$data_dir/premonition/agent-output.schema.json"
install -m 0644 -- "$project_root/manifest.json" "$plugin_dir/manifest.json"
install -m 0644 -- "$project_root/omarchy-plugin/BarWidget.qml" "$plugin_dir/omarchy-plugin/BarWidget.qml"
install -m 0644 -- "$project_root/omarchy-plugin/PremonitionSurface.qml" "$plugin_dir/omarchy-plugin/PremonitionSurface.qml"
install -m 0644 -- "$project_root/systemd/premonition.service" "$user_units/premonition.service"

{
  printf 'version = 1\n'
  printf 'git_binary = "%s"\n\n' "$git_binary"
  printf '[[repositories]]\n'
  printf 'id = "%s"\n' "$repo_id"
  printf 'label = "%s"\n' "$repo_label"
  printf 'path = "%s"\n' "$repo_path"
} >"$config_dir/repositories.toml"
chmod 0600 "$config_dir/repositories.toml"

systemctl --user daemon-reload
systemctl --user enable --now premonition.service
omarchy-shell shell rescanPlugins

printf 'Installed. Review and enable io.github.tcballard.premonition in Setup > Plugins.\n'
