# hitconn-cli

Unified HITSZ Connect command-line client. The same `hitconn` binary can run a
Linux tunnel locally or orchestrate another Linux machine over the user's
existing OpenSSH configuration. A target can start empty and does not need
outbound Internet access: the orchestrator downloads, verifies, and uploads the
matching release artifact.

Linux is the current tunnel runtime. macOS and desktop Linux can orchestrate
remote targets. Windows uses the same command and protocol design, with full
packaging and acceptance planned after the Linux workflow is stable.

## Install

On Linux or macOS, install the latest stable CLI with:

```console
curl -fsSL https://hitconn.yinmo.site/install.sh | sh
```

Ordinary users are installed to `~/.local/bin`; root is installed to
`/usr/local/bin`. Override the destination without editing shell startup files:

```console
curl -fsSL https://hitconn.yinmo.site/install.sh | HITCONN_INSTALL_DIR="$HOME/bin" sh
```

The bootstrap selects the current architecture, verifies the adjacent SHA-256
file, and atomically installs only the `hitconn` binary. It does not log in,
install a service, or change trust. To inspect it before execution, download
[`install.sh`](https://hitconn.yinmo.site/install.sh) first. Later CLI downloads
use the signed release manifest, with public GitHub Releases as the automatic
fallback.

Build from source:

```console
git clone https://github.com/YinMo19/hitconn-cli.git
cd hitconn-cli
cargo build --release
./target/release/hitconn --help
```

## Local Linux tunnel

Terminal authentication is the default. The password and optional six-digit
SMS code are read without echo, kept only in memory, and never passed through
arguments, environment variables, or logs:

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
service operations, and either systemd-resolved or resolvconf for tunnel DNS.
The explicit `hitconn login --fallback` browser path additionally needs
`certutil` from `libnss3-tools` or `nss-tools`.

While connected, a loopback-only DNS stub serves the controller's bounded
address mappings for authorized exact and wildcard domains. Other names are
forwarded to the controller-provided tunnel DNS. The resolver registration is
removed when the tunnel stops; ordinary DNS answers never create VPN routes or
expand Core's authorization policy.

The Linux runtime uses the reviewed official HITSZ macOS aTrust identity for
controller requests because HITSZ does not publish a usable Linux client
profile. Authentication and tunneling still run entirely on the Linux host;
this compatibility setting affects only the controller's client classification.

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
uploads it atomically under `~/.cache/hitconn`. If authentication is needed,
it first asks an existing local CLI session for a one-time `sidTicket`; on
macOS it can also use the current HITSZ Connect Keychain session. The ticket is
sent only on SSH stdin and redeemed into a distinct remote device session, so
cookies and SIDs are never copied. When no local session is available, hitconn
allocates a remote TTY and collects the account, password, and optional SMS
code directly on the target process. No local CLI installation is required on
the target before the first command.

`--fallback` explicitly selects remote browser authentication. It opens a
private Chrome/Chromium/Edge/Brave profile whose controller and CAS traffic
uses a loopback SSH SOCKS proxy. The WebAgent callback bypasses that proxy and
travels over an SSH local forward into the target's Core. This keeps the
browser and remote tunnel on the same network origin without changing the
system proxy. The isolated browser identifies itself as an aTrust tray only to
trigger the official WebAgent ticket handoff. The temporary profile and trust
are removed after login; passwords, cookies, `sidTicket`, and the WebAgent
private key are never printed or copied into persistent CLI state on the
orchestrator.

Remote `connect` is transient. Persistence is explicit:

```console
hitconn remote my-server connect --install
hitconn remote my-server service restart
hitconn remote my-server update
```

After `connect --install`, the same binary is available on the target as
`/usr/local/bin/hitconn`. You can then SSH to the machine and operate it without
the orchestrator, including a completely target-local authentication flow:

```console
ssh my-server
hitconn login
hitconn service start
hitconn status
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
hitconn connect [--install] [--fallback [--no-open]]
hitconn disconnect
hitconn login [--force] [--fallback [--no-open]]
hitconn logout
hitconn status [--watch] [--json]
hitconn logs [--follow] [--lines COUNT]
hitconn resources [--search QUERY] [--json]
hitconn doctor [--json]
hitconn service install|uninstall|start|restart|stop
hitconn update check|apply
hitconn config path|show|set|unset
hitconn completion <shell> [install]

hitconn remote TARGET connect [--install] [--upgrade] [--artifact PATH] [--fallback [--no-open]]
hitconn remote TARGET login [--force] [--fallback [--no-open]]
hitconn remote TARGET disconnect|logout|status|logs|resources|doctor
hitconn remote TARGET deploy|update [--artifact PATH]
hitconn remote TARGET service install|uninstall|start|restart|stop
hitconn remote TARGET purge [--yes]
```

Print completion for direct evaluation, or install the initialization block
into `~/.zshrc` once:

```console
eval "$(hitconn completion zsh)"
hitconn completion zsh install
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

Real credential/SMS entry and tunnel acceptance remain user-operated. This
project is licensed under GPL-3.0-only; see [LICENSE](LICENSE).
