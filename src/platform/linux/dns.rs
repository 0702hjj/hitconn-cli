use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use hitconn_core::{DnsOverride, TunnelNetworkSettings};

const DNS_PORT: u16 = 53;
const DNS_WORKERS: usize = 4;
const DNS_TCP_WORKERS: usize = 2;
const MAX_DNS_PACKET_BYTES: usize = 4_096;
const RESOLVCONF_RECORD: &str = "tun.hitconn0";
const TUNNEL_INTERFACE: &str = "hitconn0";

pub struct DnsManager {
    resolver: Option<ResolverRegistration>,
    proxy: Option<DnsProxy>,
}

impl DnsManager {
    pub fn start(settings: &TunnelNetworkSettings) -> Result<Option<Self>> {
        if settings.dns_servers.is_empty() {
            if settings.dns_overrides.is_empty() {
                return Ok(None);
            }
            bail!("controller supplied DNS overrides without a tunnel DNS server");
        }
        let upstreams = settings
            .dns_servers
            .iter()
            .map(|server| {
                server
                    .parse::<IpAddr>()
                    .map(|address| SocketAddr::new(address, DNS_PORT))
                    .context("Core returned an invalid tunnel DNS server")
            })
            .collect::<Result<Vec<_>>>()?;
        let proxy = DnsProxy::start(upstreams, &settings.dns_overrides)?;
        let resolver = ResolverRegistration::install(proxy.address().ip())?;
        Ok(Some(Self {
            resolver: Some(resolver),
            proxy: Some(proxy),
        }))
    }
}

impl Drop for DnsManager {
    fn drop(&mut self) {
        self.resolver.take();
        self.proxy.take();
    }
}

struct DnsProxy {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    udp_workers: Vec<JoinHandle<()>>,
    tcp_workers: Vec<JoinHandle<()>>,
}

impl DnsProxy {
    fn start(upstreams: Vec<SocketAddr>, overrides: &[DnsOverride]) -> Result<Self> {
        let (udp_socket, tcp_listener, address) = bind_stub()?;
        udp_socket.set_read_timeout(Some(Duration::from_millis(250)))?;
        tcp_listener.set_nonblocking(true)?;
        let udp_socket = Arc::new(udp_socket);
        let tcp_listener = Arc::new(tcp_listener);
        let stop = Arc::new(AtomicBool::new(false));
        let overrides = Arc::new(OverrideCatalog::new(overrides)?);
        let upstreams = Arc::new(upstreams);
        let sequence = Arc::new(AtomicUsize::new(0));
        let udp_workers = (0..DNS_WORKERS)
            .map(|_| {
                let socket = Arc::clone(&udp_socket);
                let stop = Arc::clone(&stop);
                let overrides = Arc::clone(&overrides);
                let upstreams = Arc::clone(&upstreams);
                let sequence = Arc::clone(&sequence);
                thread::spawn(move || serve_dns(socket, stop, overrides, upstreams, sequence))
            })
            .collect();
        let tcp_workers = (0..DNS_TCP_WORKERS)
            .map(|_| {
                let listener = Arc::clone(&tcp_listener);
                let stop = Arc::clone(&stop);
                let overrides = Arc::clone(&overrides);
                let upstreams = Arc::clone(&upstreams);
                let sequence = Arc::clone(&sequence);
                thread::spawn(move || serve_tcp_dns(listener, stop, overrides, upstreams, sequence))
            })
            .collect();
        Ok(Self {
            address,
            stop,
            udp_workers,
            tcp_workers,
        })
    }

    fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for DnsProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for _ in 0..self.udp_workers.len() {
            let _ = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
                .and_then(|socket| socket.send_to(&[0], self.address));
        }
        for _ in 0..self.tcp_workers.len() {
            if let Ok(stream) = TcpStream::connect_timeout(&self.address, Duration::from_secs(1)) {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
        for worker in self.udp_workers.drain(..) {
            let _ = worker.join();
        }
        for worker in self.tcp_workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[derive(Clone)]
struct OverrideEntry {
    wildcard: bool,
    addresses: Vec<Ipv4Addr>,
}

struct OverrideCatalog(BTreeMap<String, OverrideEntry>);

impl OverrideCatalog {
    fn new(overrides: &[DnsOverride]) -> Result<Self> {
        let entries = overrides
            .iter()
            .map(|entry| {
                let addresses = entry
                    .addresses
                    .iter()
                    .map(|address| {
                        address
                            .parse::<Ipv4Addr>()
                            .context("Core returned a non-IPv4 DNS override")
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok((
                    entry.domain.trim_end_matches('.').to_ascii_lowercase(),
                    OverrideEntry {
                        wildcard: entry.wildcard,
                        addresses,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok(Self(entries))
    }

    fn find(&self, domain: &str) -> Option<&OverrideEntry> {
        self.0.get(domain).or_else(|| {
            self.0.iter().find_map(|(base, entry)| {
                (entry.wildcard
                    && domain
                        .strip_suffix(base)
                        .is_some_and(|prefix| prefix.ends_with('.')))
                .then_some(entry)
            })
        })
    }
}

fn bind_stub() -> Result<(UdpSocket, TcpListener, SocketAddr)> {
    [53_u8, 54, 55]
        .into_iter()
        .find_map(|last| {
            let address = SocketAddr::from(([127, 0, 0, last], DNS_PORT));
            let udp_socket = UdpSocket::bind(address).ok()?;
            let tcp_listener = TcpListener::bind(address).ok()?;
            Some((udp_socket, tcp_listener, address))
        })
        .context("cannot bind a loopback DNS stub; port 53 is already in use")
}

fn serve_dns(
    socket: Arc<UdpSocket>,
    stop: Arc<AtomicBool>,
    overrides: Arc<OverrideCatalog>,
    upstreams: Arc<Vec<SocketAddr>>,
    sequence: Arc<AtomicUsize>,
) {
    let mut packet = [0_u8; MAX_DNS_PACKET_BYTES];
    while !stop.load(Ordering::Acquire) {
        let Ok((length, peer)) = socket.recv_from(&mut packet) else {
            continue;
        };
        if stop.load(Ordering::Acquire) {
            break;
        }
        let query = &packet[..length];
        let response = override_response(query, &overrides)
            .or_else(|| forward_query(query, &upstreams, sequence.fetch_add(1, Ordering::Relaxed)));
        if let Some(response) = response {
            let _ = socket.send_to(&response, peer);
        }
    }
}

fn forward_query(query: &[u8], upstreams: &[SocketAddr], start: usize) -> Option<Vec<u8>> {
    (0..upstreams.len()).find_map(|offset| {
        let upstream = upstreams[(start + offset) % upstreams.len()];
        let bind = if upstream.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket = UdpSocket::bind(bind).ok()?;
        socket.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        socket.connect(upstream).ok()?;
        socket.send(query).ok()?;
        let mut response = vec![0_u8; MAX_DNS_PACKET_BYTES];
        let length = socket.recv(&mut response).ok()?;
        if response.get(..2) != query.get(..2) {
            return None;
        }
        response.truncate(length);
        Some(response)
    })
}

fn serve_tcp_dns(
    listener: Arc<TcpListener>,
    stop: Arc<AtomicBool>,
    overrides: Arc<OverrideCatalog>,
    upstreams: Arc<Vec<SocketAddr>>,
    sequence: Arc<AtomicUsize>,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let _ = handle_tcp_query(
                    &mut stream,
                    &overrides,
                    &upstreams,
                    sequence.fetch_add(1, Ordering::Relaxed),
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }
}

fn handle_tcp_query(
    stream: &mut TcpStream,
    overrides: &OverrideCatalog,
    upstreams: &[SocketAddr],
    start: usize,
) -> Option<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .ok()?;
    let query = read_tcp_packet(stream)?;
    let response = override_response(&query, overrides)
        .or_else(|| forward_tcp_query(&query, upstreams, start))?;
    stream
        .write_all(&u16::try_from(response.len()).ok()?.to_be_bytes())
        .ok()?;
    stream.write_all(&response).ok()?;
    Some(())
}

fn forward_tcp_query(query: &[u8], upstreams: &[SocketAddr], start: usize) -> Option<Vec<u8>> {
    (0..upstreams.len()).find_map(|offset| {
        let upstream = upstreams[(start + offset) % upstreams.len()];
        let mut stream = TcpStream::connect_timeout(&upstream, Duration::from_secs(2)).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .ok()?;
        stream
            .write_all(&u16::try_from(query.len()).ok()?.to_be_bytes())
            .ok()?;
        stream.write_all(query).ok()?;
        let response = read_tcp_packet(&mut stream)?;
        (response.get(..2) == query.get(..2)).then_some(response)
    })
}

fn read_tcp_packet(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut length = [0_u8; 2];
    stream.read_exact(&mut length).ok()?;
    let length = usize::from(u16::from_be_bytes(length));
    if length == 0 || length > MAX_DNS_PACKET_BYTES {
        return None;
    }
    let mut packet = vec![0_u8; length];
    stream.read_exact(&mut packet).ok()?;
    Some(packet)
}

fn override_response(query: &[u8], overrides: &OverrideCatalog) -> Option<Vec<u8>> {
    let question = parse_question(query)?;
    let entry = overrides.find(&question.domain)?;
    let addresses = (question.kind == 1 && question.class == 1)
        .then_some(entry.addresses.as_slice())
        .unwrap_or_default();
    let flags = 0x8080 | (u16::from_be_bytes([query[2], query[3]]) & 0x0100);
    let mut response = Vec::with_capacity(question.end + addresses.len() * 16);
    response.extend_from_slice(&query[..2]);
    response.extend_from_slice(&flags.to_be_bytes());
    response.extend_from_slice(&1_u16.to_be_bytes());
    response.extend_from_slice(&(addresses.len() as u16).to_be_bytes());
    response.extend_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&query[12..question.end]);
    for address in addresses {
        response.extend_from_slice(&[0xc0, 0x0c]);
        response.extend_from_slice(&1_u16.to_be_bytes());
        response.extend_from_slice(&1_u16.to_be_bytes());
        response.extend_from_slice(&60_u32.to_be_bytes());
        response.extend_from_slice(&4_u16.to_be_bytes());
        response.extend_from_slice(&address.octets());
    }
    Some(response)
}

struct Question {
    domain: String,
    kind: u16,
    class: u16,
    end: usize,
}

fn parse_question(packet: &[u8]) -> Option<Question> {
    if packet.len() < 17 || u16::from_be_bytes([packet[4], packet[5]]) != 1 {
        return None;
    }
    let mut labels = Vec::new();
    let mut cursor = 12;
    loop {
        let length = *packet.get(cursor)? as usize;
        cursor += 1;
        if length == 0 {
            break;
        }
        if length > 63 || cursor.checked_add(length)? > packet.len() {
            return None;
        }
        labels.push(std::str::from_utf8(&packet[cursor..cursor + length]).ok()?);
        cursor += length;
    }
    let end = cursor.checked_add(4)?;
    if end > packet.len() {
        return None;
    }
    Some(Question {
        domain: labels.join(".").to_ascii_lowercase(),
        kind: u16::from_be_bytes([packet[cursor], packet[cursor + 1]]),
        class: u16::from_be_bytes([packet[cursor + 2], packet[cursor + 3]]),
        end,
    })
}

enum ResolverRegistration {
    SystemdResolved,
    Resolvconf,
}

impl ResolverRegistration {
    fn install(server: IpAddr) -> Result<Self> {
        let server = server.to_string();
        let dns = Command::new("resolvectl")
            .args(["dns", TUNNEL_INTERFACE, &server])
            .output();
        let domain = Command::new("resolvectl")
            .args(["domain", TUNNEL_INTERFACE, "~."])
            .output();
        if dns.is_ok_and(|output| output.status.success())
            && domain.is_ok_and(|output| output.status.success())
        {
            return Ok(Self::SystemdResolved);
        }
        let _ = Command::new("resolvectl")
            .args(["revert", TUNNEL_INTERFACE])
            .output();

        let mut child = Command::new("resolvconf")
            .args(["-a", RESOLVCONF_RECORD])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("neither systemd-resolved nor resolvconf can install tunnel DNS")?;
        writeln!(
            child
                .stdin
                .take()
                .context("resolvconf did not accept DNS configuration input")?,
            "nameserver {server}"
        )?;
        if !child.wait()?.success() {
            bail!("resolvconf rejected the tunnel DNS configuration");
        }
        Ok(Self::Resolvconf)
    }
}

impl Drop for ResolverRegistration {
    fn drop(&mut self) {
        let mut command = match self {
            Self::SystemdResolved => {
                let mut command = Command::new("resolvectl");
                command.args(["revert", TUNNEL_INTERFACE]);
                command
            }
            Self::Resolvconf => {
                let mut command = Command::new("resolvconf");
                command.args(["-d", RESOLVCONF_RECORD]);
                command
            }
        };
        let _ = command.output();
    }
}
