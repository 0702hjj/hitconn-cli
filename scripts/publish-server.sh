#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 SSH_TARGET VERSION ARTIFACT_DIRECTORY MANIFEST_JSON" >&2
  exit 2
fi

ssh_target=$1
version=$2
artifact_directory=$3
manifest=$4
repository_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
installer="$repository_root/install.sh"

if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || [[ ! -f $manifest ]] || [[ ! -f $installer ]]; then
  echo "version or manifest is invalid" >&2
  exit 2
fi

targets=(
  aarch64-apple-darwin
  x86_64-apple-darwin
  x86_64-unknown-linux-gnu
  aarch64-unknown-linux-gnu
)
release_directory="/srv/hitconn-downloads/releases/v$version"
stable_directory=/srv/hitconn-downloads/stable
ssh "$ssh_target" mkdir -p "$release_directory" "$stable_directory"

for target in "${targets[@]}"; do
  name="hitconn-$target"
  source_path="$artifact_directory/$name"
  remote_path="$release_directory/$name"
  if [[ ! -f $source_path ]]; then
    echo "missing release artifact: $source_path" >&2
    exit 2
  fi
  scp "$source_path" "$ssh_target:$remote_path.tmp"
  ssh "$ssh_target" chmod 0755 "$remote_path.tmp"
  local_sha=$(shasum -a 256 "$source_path" | cut -d ' ' -f 1)
  remote_sha=$(ssh "$ssh_target" sha256sum "$remote_path.tmp" | cut -d ' ' -f 1)
  if [[ $local_sha != "$remote_sha" ]]; then
    echo "uploaded artifact digest mismatch: $name" >&2
    exit 1
  fi
  ssh "$ssh_target" mv -f "$remote_path.tmp" "$remote_path"
  checksum=$(mktemp)
  printf '%s  %s\n' "$local_sha" "$name" > "$checksum"
  scp "$checksum" "$ssh_target:$remote_path.sha256.tmp"
  rm -f -- "$checksum"
  ssh "$ssh_target" chmod 0644 "$remote_path.sha256.tmp"
  ssh "$ssh_target" mv -f "$remote_path.sha256.tmp" "$remote_path.sha256"
done

for target in "${targets[@]}"; do
  name="hitconn-$target"
  stable_path="$stable_directory/$name"
  release_path="../releases/v$version/$name"
  ssh "$ssh_target" ln -sfn "$release_path" "$stable_path.tmp"
  ssh "$ssh_target" mv -Tf "$stable_path.tmp" "$stable_path"
  ssh "$ssh_target" ln -sfn "$release_path.sha256" "$stable_path.sha256.tmp"
  ssh "$ssh_target" mv -Tf "$stable_path.sha256.tmp" "$stable_path.sha256"
done

scp "$installer" "$ssh_target:/srv/hitconn-downloads/install.sh.tmp"
ssh "$ssh_target" chmod 0644 /srv/hitconn-downloads/install.sh.tmp
ssh "$ssh_target" mv -f /srv/hitconn-downloads/install.sh.tmp /srv/hitconn-downloads/install.sh
scp "$manifest" "$ssh_target:$stable_directory/manifest.json.tmp"
ssh "$ssh_target" chmod 0644 "$stable_directory/manifest.json.tmp"
ssh "$ssh_target" mv -f "$stable_directory/manifest.json.tmp" "$stable_directory/manifest.json"
echo "Published hitconn $version to $ssh_target"
