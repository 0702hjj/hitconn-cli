#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: HITCONN_SIGNING_KEY=/path/to/key.pem $0 PAYLOAD_JSON OUTPUT_JSON" >&2
  exit 2
fi

payload=$1
output=$2
signing_key=${HITCONN_SIGNING_KEY:?HITCONN_SIGNING_KEY must name an Ed25519 PEM private key}

if [[ ! -f "$payload" || ! -f "$signing_key" ]]; then
  echo "payload or signing key does not exist" >&2
  exit 2
fi

openssl_bin=${HITCONN_OPENSSL:-openssl}
if [[ -x /opt/homebrew/opt/openssl@3/bin/openssl && -z ${HITCONN_OPENSSL:-} ]]; then
  openssl_bin=/opt/homebrew/opt/openssl@3/bin/openssl
fi

temporary_directory=$(mktemp -d)
trap 'rm -rf -- "$temporary_directory"' EXIT
signature="$temporary_directory/signature.bin"

"$openssl_bin" pkeyutl -sign -rawin -inkey "$signing_key" -in "$payload" -out "$signature"
signed_payload=$("$openssl_bin" base64 -A -in "$payload")
encoded_signature=$("$openssl_bin" base64 -A -in "$signature")
jq -n --arg signed "$signed_payload" --arg signature "$encoded_signature" \
  '{signed: $signed, signature: $signature}' > "$output"
