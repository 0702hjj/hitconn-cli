# Unified CLI architecture

`hitconn` is one binary with two composable roles:

- the Linux runtime owns the TUN adapter, routes, DNS, authentication state,
  and systemd lifecycle;
- an orchestrator owns artifact acquisition and OpenSSH transport; only the
  explicit browser fallback also owns temporary user trust and an isolated
  remote-login browser profile.

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
   usable. If not, run the terminal login on a target-side SSH TTY. The account,
   password, and optional SMS code go directly to the target process and never
   enter the orchestrator's process or machine protocol.
6. When `--fallback` is explicit, select a WebAgent port free at both ends and
   start a short-lived WebAgent identity in remote memory. The same SSH process
   exposes its port with `-L` and a loopback SOCKS endpoint with `-D`.
7. Validate and temporarily trust only the WebAgent's `127.0.0.1` non-CA leaf,
   then launch a private Chromium-family profile. Controller/CAS requests and
   DNS use the SOCKS path so authentication has the target's network origin;
   loopback bypass rules send the WebAgent callback into the `-L` forward. Its
   user agent retains the installed Chromium version and adds the official
   `SPCClientType aTrustTray` marker required for the portal handoff. Chromium's
   local-network check is disabled only for this disposable profile; the remote
   WebAgent still enforces its strict Origin allowlist. Machine-wide proxy
   settings are never changed.
8. The portal sends `sidTicket` through the TLS forward directly to the remote
   Core. The ticket, session cookies, and private key never enter local CLI
   output. The browser profile and temporary trust are removed after success,
   cancellation, or timeout.
9. Start a transient service, or install and enable it only when `--install`
   was explicitly requested.

Machine protocol stdout is versioned NDJSON. Human diagnostics and daemon logs
use stderr. Read-only remote commands never deploy or silently update a target.

## Release manifest

The source URLs are configurable. `hitconn.yinmo.site` serves the primary
manifest and immutable artifacts; GitHub Releases serves an automatic fallback
manifest and per-artifact mirror using the same signed envelope.
The envelope contains base64-encoded signed payload bytes and an Ed25519
signature. The payload names each target artifact, URL, byte size, and SHA-256
digest, plus compatible agent protocol versions. Download credentials remain
local and are neither persisted in config nor forwarded over SSH.
