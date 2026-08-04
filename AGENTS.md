# hitconn-cli agent guide

This repository owns the headless frontend, remote orchestrator, and platform
adapters for `hitconn-core`. One `hitconn` binary serves both local and remote
roles. Linux is the tunnel runtime; Linux, macOS, and later Windows may act as
browser-capable orchestrators without gaining tunnel privileges locally.

## Architecture and security

- Core owns authentication protocol, cookies/CSRF, resource validation, route
  generation, tunnel authorization, and TLS identity. The CLI must consume
  `TunnelNetworkSettings` and never reinterpret raw controller policy.
- Linux owns the TUN device, application of Core-produced routes and DNS,
  systemd lifecycle, journal access, and private on-disk session storage.
- Browser login must keep passwords and factors in the system browser. Trust
  only the non-CA `127.0.0.1` WebAgent leaf in the current user's browser
  stores. Never install or trust a Sangfor root CA.
- Remote browser login uses an SSH local forward on the same official WebAgent
  port at both ends. Its short-lived private key exists only in the remote
  process; the orchestrator receives and validates only the leaf certificate.
- Use the system OpenSSH client and preserve the user's host-key policy,
  aliases, ProxyJump, agent, hardware keys, and MFA. Never disable host-key
  checking, collect a sudo password, or copy local credentials to a target.
- A target does not need outbound Internet access. Download and verify release
  artifacts on the orchestrator, then upload atomically over SSH. Keep the
  source configurable through a signed generic manifest. The first-party
  `hitconn.yinmo.site` service is primary and GitHub Releases is the automatic
  fallback; GitHub tokens never leave the orchestrator.
- Machine-facing commands use versioned NDJSON on stdout and diagnostics on
  stderr. They are hidden, bounded operations rather than an arbitrary command
  channel. Never emit cookies, SIDs, tickets, private keys, or certificate
  bodies in logs; a login-ready certificate is the sole protocol exception.
- Remote `status`, `logs`, and `doctor` are read-only and must not deploy or
  upgrade. `remote connect` is transient by default; persistence requires
  `--install`. Active services upgrade only through explicit update commands,
  with a retained previous artifact and rollback on failed activation.
- Publish immutable versioned artifacts before atomically replacing the stable
  manifest. Verify hashes after upload, never place the release signing key on
  the download server, and keep both first-party and GitHub mirrors usable.
- Never log or print cookies, tickets, SIDs, signing keys, challenges,
  certificate bodies, or downloaded private content.
- Keep source files below 500 lines and keep platform-specific code isolated.

## Required checks

Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, host
Release builds, and x86_64/aarch64 Linux release builds before publishing.
Exercise the SSH protocol against a disposable local target when available.
Real login and tunnel acceptance must be performed by the user; do not
automate their browser or desktop.

## Git workflow

Commit focused changes and push the current branch to its configured upstream.
Never include credentials, runtime state, generated WebAgent identities, or
signed URLs.
