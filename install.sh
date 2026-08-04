#!/bin/sh
set -eu

umask 077

download_base=${HITCONN_DOWNLOAD_BASE:-https://hitconn.yinmo.site}
case "$download_base" in
    https://*) ;;
    *)
        echo "error: HITCONN_DOWNLOAD_BASE must use HTTPS" >&2
        exit 1
        ;;
esac
download_base=${download_base%/}

kernel=$(uname -s)
machine=$(uname -m)
case "$kernel:$machine" in
    Linux:x86_64 | Linux:amd64)
        target=x86_64-unknown-linux-gnu
        ;;
    Linux:aarch64 | Linux:arm64)
        target=aarch64-unknown-linux-gnu
        ;;
    Darwin:x86_64 | Darwin:amd64)
        target=x86_64-apple-darwin
        ;;
    Darwin:arm64 | Darwin:aarch64)
        target=aarch64-apple-darwin
        ;;
    *)
        echo "error: unsupported platform $kernel/$machine" >&2
        exit 1
        ;;
esac

if [ -n "${HITCONN_INSTALL_DIR:-}" ]; then
    install_dir=$HITCONN_INSTALL_DIR
elif [ "$(id -u)" -eq 0 ]; then
    install_dir=/usr/local/bin
else
    install_dir=${HOME:?HOME is not set}/.local/bin
fi

for command_name in curl mktemp awk grep; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "error: $command_name is required" >&2
        exit 1
    fi
done
if command -v sha256sum >/dev/null 2>&1; then
    calculate_sha256() {
        sha256sum "$1" | awk '{ print $1 }'
    }
elif command -v shasum >/dev/null 2>&1; then
    calculate_sha256() {
        shasum -a 256 "$1" | awk '{ print $1 }'
    }
else
    echo "error: sha256sum or shasum is required" >&2
    exit 1
fi

temporary_directory=$(mktemp -d "${TMPDIR:-/tmp}/hitconn-install.XXXXXX")
temporary_destination=
cleanup() {
    rm -rf -- "$temporary_directory"
    if [ -n "$temporary_destination" ]; then
        rm -f -- "$temporary_destination"
    fi
}
trap cleanup EXIT HUP INT TERM

artifact_name=hitconn-$target
artifact=$temporary_directory/$artifact_name
checksum=$temporary_directory/$artifact_name.sha256
curl -fsSL --proto '=https' --tlsv1.2 --retry 3 \
    "$download_base/stable/$artifact_name.sha256" -o "$checksum"
curl -fsSL --proto '=https' --tlsv1.2 --retry 3 \
    "$download_base/stable/$artifact_name" -o "$artifact"

expected=$(awk 'NR == 1 { print $1 }' "$checksum")
if ! printf '%s\n' "$expected" | grep -Eq '^[0-9a-fA-F]{64}$'; then
    echo "error: download service returned an invalid SHA-256 checksum" >&2
    exit 1
fi
actual=$(calculate_sha256 "$artifact")
if [ "$actual" != "$expected" ]; then
    echo "error: downloaded hitconn SHA-256 does not match" >&2
    exit 1
fi

mkdir -p -- "$install_dir"
if [ ! -w "$install_dir" ]; then
    echo "error: $install_dir is not writable" >&2
    echo "set HITCONN_INSTALL_DIR to a writable directory or run this installer as root" >&2
    exit 1
fi
temporary_destination=$install_dir/.hitconn.tmp.$$
cp -- "$artifact" "$temporary_destination"
chmod 0755 "$temporary_destination"
mv -f -- "$temporary_destination" "$install_dir/hitconn"
temporary_destination=

version=$($install_dir/hitconn --version 2>/dev/null || true)
printf 'Installed %s to %s/hitconn\n' "${version:-hitconn}" "$install_dir"
case ":${PATH:-}:" in
    *:"$install_dir":*) ;;
    *) printf 'Add %s to PATH before running hitconn.\n' "$install_dir" ;;
esac
