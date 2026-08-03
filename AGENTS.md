# hitconn-cli agent guide

This repository owns the headless frontend and platform adapters for
`hitconn-core`. Linux is the supported platform; keep macOS and Windows
boundaries explicit so adapters can be added later without changing commands.

## Architecture and security

- Core owns authentication protocol, cookies/CSRF, resource validation, route
  generation, tunnel authorization, and TLS identity. The CLI must consume
  `TunnelNetworkSettings` and never reinterpret raw controller policy.
- Linux owns the TUN device, application of Core-produced routes and DNS,
  systemd lifecycle, journal access, and private on-disk session storage.
- Browser login must keep passwords and factors in the system browser. Trust
  only the non-CA `127.0.0.1` WebAgent leaf in the current user's browser
  stores. Never install or trust a Sangfor root CA.
- Never log or print cookies, tickets, SIDs, signing keys, challenges,
  certificate bodies, or downloaded private content.
- Keep source files below 500 lines and keep platform-specific code isolated.

## Required checks

Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and a
Linux release build before publishing. Real login and tunnel acceptance must
be performed by the user on Linux; do not automate their browser or desktop.

## Git workflow

Commit focused changes and push the current branch to its configured upstream.
Never include credentials, runtime state, generated WebAgent identities, or
signed URLs.

