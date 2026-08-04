# hitconn-cli

Unified HITSZ Connect command-line client. The same `hitconn` binary can run a
Linux tunnel locally or orchestrate another Linux machine over the user's
existing OpenSSH configuration. A target can start empty and does not need
outbound Internet access: the orchestrator downloads, verifies, and uploads the
matching release artifact.

Linux is the current tunnel runtime. macOS and desktop Linux can act as browser
orchestrators. Windows uses the same command and protocol design, with full
packaging and acceptance planned after the Linux workflow is stable.

## Install

Download the artifact for the local machine from the latest release, make it
executable on Unix, and place it on `PATH`. The repository is private for now,
so automatic GitHub downloads reuse the local `gh auth token`; that token is
never saved by hitconn, printed, or sent to a target.

Build from source with the existing Git credentials:

```console
git clone https://github.com/YinMo19/hitconn-cli.git
cd hitconn-cli
cargo build --release
./target/release/hitconn --help
```

## Local Linux tunnel

Authentication always stays in the default browser:

```console
hitconn login
hitconn connect
hitconn status
hitconn logs --follow
hitconn disconnect
```

`connect` starts a transient systemd unit by default. To install the current
binary under `/usr/local/bin`, enable it at boot, and start it immediately:

```console
hitconn connect --install
hitconn service restart
hitconn service stop
hitconn service uninstall
```

`logout` stops the service and removes saved authentication and local WebAgent
trust. It does not uninstall the binary or persistent unit.

Linux requirements are systemd, `/dev/net/tun`, `iproute2`, root access for
service operations, and `certutil` from `libnss3-tools` or `nss-tools` for
browser trust. `systemd-resolved` is recommended for tunnel DNS.

## Remote Linux tunnel

`TARGET` is any OpenSSH host syntax or alias. Existing `~/.ssh/config`,
ProxyJump, ControlMaster, agent, hardware keys, MFA, and host-key policy are
preserved.

```console
hitconn remote my-server doctor
hitconn remote my-server connect
hitconn remote my-server status
hitconn remote my-server logs --follow
hitconn remote my-server disconnect
```

On the first `connect`, hitconn inspects the remote OS/architecture, downloads
the latest signed Linux artifact locally, verifies its size and SHA-256, and
uploads it atomically under `~/.cache/hitconn`. If authentication is needed, it
opens the portal in the orchestrator's browser while the one-time callback
travels over an SSH local forward into the target's Core. Passwords, cookies,
`sidTicket`, and the WebAgent private key never leave their owning side.

Remote `connect` is transient. Persistence is explicit:

```console
hitconn remote my-server connect --install
hitconn remote my-server service restart
hitconn remote my-server update
```

`status`, `logs`, `resources`, and `doctor` never deploy or silently upgrade.
Use `deploy`, `connect --upgrade`, or `update` explicitly. A reviewed local
artifact can replace release resolution:

```console
hitconn remote my-server deploy --artifact ./hitconn-linux-x86_64
```

To remove the remote unit, installed binary, artifact cache, session, and
runtime state:

```console
hitconn remote my-server purge
```

## Commands

```text
hitconn connect [--install] [--no-open]
hitconn disconnect
hitconn login [--force] [--no-open]
hitconn logout
hitconn status [--watch] [--json]
hitconn logs [--follow] [--lines COUNT]
hitconn resources [--search QUERY] [--json]
hitconn doctor [--json]
hitconn service install|uninstall|start|restart|stop
hitconn update check|apply
hitconn config path|show|set|unset
hitconn completion <shell>

hitconn remote TARGET connect [--install] [--upgrade] [--artifact PATH]
hitconn remote TARGET disconnect|login|logout|status|logs|resources|doctor
hitconn remote TARGET deploy|update [--artifact PATH]
hitconn remote TARGET service install|uninstall|start|restart|stop
hitconn remote TARGET purge [--yes]
```

The signed release source is generic and configurable. By default,
`https://hitconn.yinmo.site` is primary and GitHub Releases is the automatic
fallback:

```console
hitconn config set manifest_url https://downloads.example.com/hitconn/manifest.json
hitconn config set fallback_manifest_url https://mirror.example.com/manifest.json
hitconn config set channel stable
```

See [docs/architecture.md](docs/architecture.md) for trust, SSH, protocol, and
artifact boundaries.

## Development checks

```console
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build --release
cargo zigbuild --release --target x86_64-unknown-linux-gnu
cargo zigbuild --release --target aarch64-unknown-linux-gnu
```

Real browser login and tunnel acceptance remain user-operated. This project is
licensed under GPL-3.0-only; see [LICENSE](LICENSE).
