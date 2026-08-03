# Release procedure

Release artifacts are raw `hitconn` executables. Build at least
`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, and
`aarch64-unknown-linux-gnu`. Add other orchestrator artifacts without changing
the manifest schema.

The release private key must remain outside the repository with mode `0600`.
Its matching public key is embedded in `src/artifact.rs`. To rotate it, publish
a bridging CLI release before signing only with the replacement key.

Copy the four binaries into one directory using names of the form
`hitconn-TARGET`, then create the compact payload:

```console
scripts/create-manifest-payload.sh 0.2.2 \
  https://github.com/YinMo19/hitconn-cli/releases/download/v0.2.2 \
  release-assets manifest-payload.json
```

The generated payload has this shape:

```json
{"schemaVersion":1,"channel":"stable","version":"0.2.2","protocolMin":1,"protocolMax":1,"artifacts":[{"target":"x86_64-unknown-linux-gnu","url":"https://example/hitconn-x86_64-unknown-linux-gnu","size":123,"sha256":"..."}]}
```

Sign the exact payload bytes:

```console
HITCONN_SIGNING_KEY=/secure/path/signing-key.pem \
  scripts/sign-manifest.sh manifest-payload.json manifest.json
```

Upload `manifest.json` and every named artifact to the release location. The
artifact URLs may use GitHub Releases now or a first-party HTTPS endpoint later.
Test `hitconn update check` and a clean `hitconn remote TARGET deploy` before
marking the release current. Never upload the payload private key, auth token,
session state, or generated WebAgent identity.
