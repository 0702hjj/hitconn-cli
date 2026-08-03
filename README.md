# hitconn-cli

Headless HITSZ Connect client for Linux. It reuses `hitconn-core` for browser
authentication, validated resource policy, route generation, tunnel TLS, and
packet authorization, while this repository owns the Linux TUN interface and
systemd lifecycle.

The command surface is intentionally platform-neutral so macOS and Windows
service adapters can be added later. The current release supports Linux with
systemd only.

## Requirements

- A systemd-based Linux distribution;
- `iproute2` for route installation;
- `/dev/net/tun` and root access for service operations;
- `certutil` from `libnss3-tools` on Debian/Ubuntu or `nss-tools` on Fedora;
- `xdg-open` or `gio` to open the system browser;
- systemd-resolved is recommended for tunnel DNS.

Authentication stays in the default browser. The CLI stores only the Core
session snapshot under `${XDG_STATE_HOME:-$HOME/.local/state}/hitconn` with
user-only permissions. It imports only the generated non-CA `127.0.0.1`
WebAgent leaf into the current user's NSS browser databases; it never installs
a Sangfor root CA.

## Build

The Core dependency is pinned to a reviewed `hitconn-core` commit. Git uses
your existing GitHub credentials to fetch that private repository:

```console
git clone https://github.com/YinMo19/hitconn-cli.git
cd hitconn-cli
cargo build --release
```

Run the Release binary directly or copy it somewhere on `PATH`:

```console
./target/release/hitconn --help
```

## Login and connect

Run browser login as the desktop user, without `sudo`:

```console
hitconn login
```

For a one-session daemon that is not enabled at boot:

```console
hitconn service start
hitconn status
hitconn logs --follow
hitconn service restart
hitconn service stop
```

When no persistent unit is installed, `service start` creates a transient
`hitconn.service` through `systemd-run`. The command asks for `sudo` only for
the privileged service operation; browser state remains owned by the desktop
user.

To install the current binary under `/usr/local/bin`, enable it at boot, and
start it immediately:

```console
hitconn service install
```

Re-running `service install` updates the installed binary and unit. Uninstall
stops the daemon and removes `/etc/systemd/system/hitconn.service` and
`/usr/local/bin/hitconn`, while preserving the saved browser session:

```console
hitconn service uninstall
```

To stop the service, remove the saved session, delete the local WebAgent
identity, and remove its browser trust entry:

```console
hitconn logout
```

## Command summary

```text
hitconn login
hitconn logout
hitconn service install|uninstall|start|restart|stop
hitconn status
hitconn logs [-f|--follow] [-n|--lines COUNT]
```

The daemon retries transient controller, network, and tunnel failures. An
expired or missing session leaves the service running in
`authentication_required` state so a later `hitconn login` can restore the
tunnel without reinstalling the service.

## Development checks

```console
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo zigbuild --release --target x86_64-unknown-linux-gnu
cargo zigbuild --release --target aarch64-unknown-linux-gnu
```

Source licensing has not yet been selected; no rights are granted beyond the
repository owner's explicit permissions.
