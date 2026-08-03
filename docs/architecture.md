# Unified CLI architecture

`hitconn` is one binary with two composable roles:

- the Linux runtime owns the TUN adapter, routes, DNS, authentication state,
  and systemd lifecycle;
- a browser-capable orchestrator owns artifact acquisition, OpenSSH transport,
  temporary user trust, and opening the local browser.

A desktop Linux machine can perform either role. macOS is initially an
orchestrator only. Windows follows the same boundary when its trust adapter is
enabled; native macOS and Windows tunnels are separate future platform work.

## Remote connection flow

1. Resolve the SSH target through the user's existing OpenSSH configuration.
2. Inspect the target OS and architecture without changing it.
3. Fetch a signed release manifest locally, verify its embedded Ed25519
   signature, download the matching artifact, and verify size and SHA-256.
4. Upload to a versioned user cache and atomically switch the `current`
   symlink. The target does not need Internet access.
5. Ask the hidden, versioned agent protocol whether the saved session is
   usable. If not, select a WebAgent port free at both ends.
6. Start a short-lived WebAgent identity in remote memory, forward the same
   port with `ssh -L`, validate and temporarily trust only its `127.0.0.1`
   non-CA leaf, then open the login URL in the local default browser.
7. The portal sends `sidTicket` through the TLS forward directly to the remote
   Core. The ticket, session cookies, and private key never enter local CLI
   output. Temporary trust is removed after success, cancellation, or timeout.
8. Start a transient service, or install and enable it only when `--install`
   was explicitly requested.

Machine protocol stdout is versioned NDJSON. Human diagnostics and daemon logs
use stderr. Read-only remote commands never deploy or silently update a target.

## Release manifest

The source URL is configurable. The default may point to GitHub Releases while
private; a future first-party download endpoint can serve the same envelope.
The envelope contains base64-encoded signed payload bytes and an Ed25519
signature. The payload names each target artifact, URL, byte size, and SHA-256
digest, plus compatible agent protocol versions. Download credentials remain
local and are neither persisted in config nor forwarded over SSH.
