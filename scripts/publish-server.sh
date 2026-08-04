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

if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || [[ ! -f $manifest ]]; then
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
done

scp "$manifest" "$ssh_target:$stable_directory/manifest.json.tmp"
ssh "$ssh_target" chmod 0644 "$stable_directory/manifest.json.tmp"
ssh "$ssh_target" mv -f "$stable_directory/manifest.json.tmp" "$stable_directory/manifest.json"
echo "Published hitconn $version to $ssh_target"
