#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 5 ]]; then
  echo "usage: $0 VERSION PRIMARY_BASE_URL MIRROR_BASE_URL ARTIFACT_DIRECTORY OUTPUT_JSON" >&2
  exit 2
fi

version=$1
base_url=${2%/}
mirror_base_url=${3%/}
artifact_directory=$4
output=$5
targets=(
  aarch64-apple-darwin
  x86_64-apple-darwin
  x86_64-unknown-linux-gnu
  aarch64-unknown-linux-gnu
)

artifacts='[]'
for target in "${targets[@]}"; do
  name="hitconn-$target"
  path="$artifact_directory/$name"
  if [[ ! -f "$path" ]]; then
    echo "missing release artifact: $path" >&2
    exit 2
  fi
  size=$(stat -f %z "$path" 2>/dev/null || stat -c %s "$path")
  if command -v shasum >/dev/null 2>&1; then
    sha256=$(shasum -a 256 "$path" | cut -d ' ' -f 1)
  else
    sha256=$(sha256sum "$path" | cut -d ' ' -f 1)
  fi
  artifact=$(jq -cn \
    --arg target "$target" \
    --arg url "$base_url/$name" \
    --arg mirror "$mirror_base_url/$name" \
    --argjson size "$size" \
    --arg sha256 "$sha256" \
    '{target:$target,url:$url,mirrors:[$mirror],size:$size,sha256:$sha256}')
  artifacts=$(jq -cn --argjson current "$artifacts" --argjson item "$artifact" \
    '$current + [$item]')
done

jq -cn \
  --arg version "$version" \
  --argjson artifacts "$artifacts" \
  '{schemaVersion:1,channel:"stable",version:$version,protocolMin:1,protocolMax:1,artifacts:$artifacts}' \
  > "$output"
