pub mod trust;

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use hitconn_core::{Ipv4Route, TunnelNetworkSettings};
use serde::Deserialize;
use tun_rs::{AsyncDevice, DeviceBuilder, Layer};

const INTERFACE_NAME: &str = "hitconn0";
const ROUTE_METRIC: &str = "42760";

pub struct NetworkAdapter {
    device: Arc<AsyncDevice>,
    routes: RouteManager,
    dns_configured: bool,
}

impl NetworkAdapter {
    pub fn create(settings: &TunnelNetworkSettings) -> Result<Self> {
        ensure_ip_available()?;
        let address = settings
            .address
            .parse::<Ipv4Addr>()
            .context("Core returned an invalid tunnel IPv4 address")?;
        let device = DeviceBuilder::new()
            .name(INTERFACE_NAME)
            .layer(Layer::L3)
            .mtu(settings.mtu)
            .ipv4(address, 32, None::<Ipv4Addr>)
            .build_async()
            .context("cannot create the Linux TUN interface; CAP_NET_ADMIN is required")?;
        let mut routes = RouteManager::default();
        if let Err(error) = routes.install(settings) {
            routes.cleanup();
            return Err(error);
        }
        let dns_configured = configure_dns(&settings.dns_servers);
        Ok(Self {
            device: Arc::new(device),
            routes,
            dns_configured,
        })
    }

    pub async fn receive(&self, buffer: &mut [u8]) -> Result<usize> {
        self.device
            .recv(buffer)
            .await
            .context("failed to read a packet from the TUN interface")
    }

    pub async fn send(&self, packet: &[u8]) -> Result<()> {
        let written = self
            .device
            .send(packet)
            .await
            .context("failed to write a packet to the TUN interface")?;
        if written != packet.len() {
            bail!(
                "TUN interface accepted only {written} of {} bytes",
                packet.len()
            );
        }
        Ok(())
    }

    pub fn apply(&mut self, settings: &TunnelNetworkSettings) -> Result<()> {
        self.routes.update(settings)?;
        if settings.dns_servers.is_empty() {
            if self.dns_configured {
                revert_dns();
            }
            self.dns_configured = false;
        } else {
            self.dns_configured = configure_dns(&settings.dns_servers);
        }
        Ok(())
    }

    pub fn cleanup(&mut self) {
        if self.dns_configured {
            revert_dns();
            self.dns_configured = false;
        }
        self.routes.cleanup();
    }
}

#[derive(Default)]
struct RouteManager {
    included: BTreeSet<String>,
    excluded: Vec<InstalledRoute>,
}

#[derive(Debug, Clone)]
struct InstalledRoute {
    cidr: String,
    gateway: Option<String>,
    device: String,
}

#[derive(Debug, Deserialize)]
struct RouteProbe {
    gateway: Option<String>,
    dev: String,
}

impl RouteManager {
    fn install(&mut self, settings: &TunnelNetworkSettings) -> Result<()> {
        for route in &settings.excluded_routes {
            let underlay = probe_underlay(route)?;
            add_underlay(&underlay)?;
            self.excluded.push(underlay);
        }
        for route in &settings.included_routes {
            add_tunnel(route)?;
            self.included.insert(route.cidr()?);
        }
        Ok(())
    }

    fn update(&mut self, settings: &TunnelNetworkSettings) -> Result<()> {
        let excluded = settings
            .excluded_routes
            .iter()
            .map(Ipv4Route::cidr)
            .collect::<hitconn_core::Result<BTreeSet<_>>>()?;
        let installed = self
            .excluded
            .iter()
            .map(|route| route.cidr.clone())
            .collect::<BTreeSet<_>>();
        if excluded != installed {
            bail!("tunnel underlay changed and requires a clean restart");
        }

        let desired = settings
            .included_routes
            .iter()
            .map(Ipv4Route::cidr)
            .collect::<hitconn_core::Result<BTreeSet<_>>>()?;
        let mut added: Vec<String> = Vec::new();
        for route in &settings.included_routes {
            let cidr = route.cidr()?;
            if self.included.contains(&cidr) {
                continue;
            }
            if let Err(error) = add_tunnel(route) {
                for cidr in added {
                    delete_tunnel(&cidr);
                }
                return Err(error);
            }
            added.push(cidr);
        }
        for cidr in self.included.difference(&desired) {
            delete_tunnel(cidr);
        }
        self.included = desired;
        Ok(())
    }

    fn cleanup(&mut self) {
        for cidr in std::mem::take(&mut self.included) {
            delete_tunnel(&cidr);
        }
        for route in std::mem::take(&mut self.excluded) {
            delete_underlay(&route);
        }
    }
}

fn probe_underlay(route: &Ipv4Route) -> Result<InstalledRoute> {
    let output = Command::new("ip")
        .args(["-json", "-4", "route", "get", &route.destination])
        .output()
        .context("cannot inspect the tunnel underlay route")?;
    if !output.status.success() {
        bail!("no usable underlay route for {}", route.destination);
    }
    let probe = serde_json::from_slice::<Vec<RouteProbe>>(&output.stdout)?
        .into_iter()
        .next()
        .context("ip route get returned no route")?;
    if probe.dev == INTERFACE_NAME {
        bail!("tunnel underlay would recursively use {INTERFACE_NAME}");
    }
    Ok(InstalledRoute {
        cidr: route.cidr()?,
        gateway: probe.gateway,
        device: probe.dev,
    })
}

fn add_tunnel(route: &Ipv4Route) -> Result<()> {
    let cidr = route.cidr()?;
    run_ip(&[
        "-4",
        "route",
        "replace",
        &cidr,
        "dev",
        INTERFACE_NAME,
        "proto",
        "static",
        "metric",
        ROUTE_METRIC,
    ])
}

fn delete_tunnel(cidr: &str) {
    let _ = run_ip(&[
        "-4",
        "route",
        "del",
        cidr,
        "dev",
        INTERFACE_NAME,
        "proto",
        "static",
        "metric",
        ROUTE_METRIC,
    ]);
}

fn add_underlay(route: &InstalledRoute) -> Result<()> {
    let mut arguments = vec!["-4", "route", "replace", route.cidr.as_str()];
    if let Some(gateway) = &route.gateway {
        arguments.extend(["via", gateway]);
    }
    arguments.extend([
        "dev",
        &route.device,
        "proto",
        "static",
        "metric",
        ROUTE_METRIC,
    ]);
    run_ip(&arguments)
}

fn delete_underlay(route: &InstalledRoute) {
    let mut arguments = vec!["-4", "route", "del", route.cidr.as_str()];
    if let Some(gateway) = &route.gateway {
        arguments.extend(["via", gateway]);
    }
    arguments.extend([
        "dev",
        &route.device,
        "proto",
        "static",
        "metric",
        ROUTE_METRIC,
    ]);
    let _ = run_ip(&arguments);
}

fn run_ip(arguments: &[&str]) -> Result<()> {
    let output = Command::new("ip").args(arguments).output()?;
    if output.status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&output.stderr);
        bail!("ip {} failed: {}", arguments.join(" "), message.trim());
    }
}

fn ensure_ip_available() -> Result<()> {
    if Command::new("ip").arg("-Version").output().is_ok() {
        Ok(())
    } else {
        bail!("the Linux `ip` command is required; install iproute2")
    }
}

fn configure_dns(servers: &[String]) -> bool {
    if servers.is_empty() {
        return false;
    }
    let mut command = Command::new("resolvectl");
    command.args(["dns", INTERFACE_NAME]);
    command.args(servers);
    let dns = command.output();
    let domain = Command::new("resolvectl")
        .args(["domain", INTERFACE_NAME, "~."])
        .output();
    let configured = dns.is_ok_and(|output| output.status.success())
        && domain.is_ok_and(|output| output.status.success());
    if !configured {
        tracing::warn!("systemd-resolved is unavailable; tunnel DNS was not installed");
    }
    configured
}

fn revert_dns() {
    let _ = Command::new("resolvectl")
        .args(["revert", INTERFACE_NAME])
        .output();
}
