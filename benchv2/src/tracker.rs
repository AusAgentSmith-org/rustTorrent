use crate::bencode::BValue;
use dashmap::DashMap;
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

struct PeerEntry {
    ip: Ipv4Addr,
    port: u16,
    last_seen: Instant,
}

struct TrackerState {
    // info_hash (20 bytes) -> list of peers
    peers: DashMap<[u8; 20], Vec<PeerEntry>>,
    announces: AtomicU64,
}

impl TrackerState {
    fn new() -> Self {
        Self {
            peers: DashMap::new(),
            announces: AtomicU64::new(0),
        }
    }

    fn announce(
        &self,
        info_hash: [u8; 20],
        peer_ip: Ipv4Addr,
        port: u16,
        event: &str,
    ) -> Vec<u8> {
        self.announces.fetch_add(1, Ordering::Relaxed);
        let now = Instant::now();

        // Clean expired and update
        let mut entry = self.peers.entry(info_hash).or_default();
        entry.retain(|p| now.duration_since(p.last_seen).as_secs() < 120);

        if event == "stopped" {
            entry.retain(|p| !(p.ip == peer_ip && p.port == port));
        } else {
            if let Some(existing) = entry.iter_mut().find(|p| p.ip == peer_ip && p.port == port) {
                existing.last_seen = now;
            } else {
                entry.push(PeerEntry {
                    ip: peer_ip,
                    port,
                    last_seen: now,
                });
            }
        }

        // Build compact peer list (excluding requester)
        let mut compact = Vec::new();
        for p in entry.iter() {
            if p.ip == peer_ip && p.port == port {
                continue;
            }
            compact.extend_from_slice(&p.ip.octets());
            compact.extend_from_slice(&p.port.to_be_bytes());
        }
        let total = entry.len();
        drop(entry);

        // Bencode response
        let mut resp = BTreeMap::new();
        resp.insert(b"interval".to_vec(), BValue::Int(30));
        resp.insert(b"min interval".to_vec(), BValue::Int(10));
        resp.insert(b"complete".to_vec(), BValue::Int(total as i64));
        resp.insert(b"incomplete".to_vec(), BValue::Int(0));
        resp.insert(b"peers".to_vec(), BValue::Bytes(compact));
        BValue::Dict(resp).encode()
    }

    fn stats(&self) -> String {
        let n_torrents = self.peers.len();
        let n_peers: usize = self.peers.iter().map(|e| e.value().len()).sum();
        format!(
            "torrents:{}\npeers:{}\nannounces:{}\n",
            n_torrents,
            n_peers,
            self.announces.load(Ordering::Relaxed),
        )
    }
}

pub async fn run(port: u16) -> anyhow::Result<()> {
    let state = Arc::new(TrackerState::new());
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("Tracker listening on 0.0.0.0:{port}");

    // Handle SIGTERM gracefully — keep running until explicitly killed
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        let state = state.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, addr, &state).await {
                                tracing::debug!("tracker conn error: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("tracker accept error: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
            _ = sigterm.recv() => {
                tracing::info!("Tracker shutting down (SIGTERM)");
                return Ok(());
            }
        }
    }
}

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    addr: SocketAddr,
    state: &TrackerState,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let request = std::str::from_utf8(&buf[..n])?;

    let first_line = request.lines().next().unwrap_or("");
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");

    let (status, body) = if path.starts_with("/announce") {
        handle_announce(path, addr, state)
    } else if path.starts_with("/health") {
        ("200 OK".to_string(), b"OK".to_vec())
    } else if path.starts_with("/stats") {
        ("200 OK".to_string(), state.stats().into_bytes())
    } else {
        ("404 Not Found".to_string(), b"Not Found".to_vec())
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(&body).await?;
    Ok(())
}

fn handle_announce(
    path: &str,
    addr: SocketAddr,
    state: &TrackerState,
) -> (String, Vec<u8>) {
    let query = path.splitn(2, '?').nth(1).unwrap_or("");
    let params = parse_query(query);

    let info_hash = match params.get("info_hash") {
        Some(ih) if ih.len() == 20 => {
            let mut arr = [0u8; 20];
            arr.copy_from_slice(ih);
            arr
        }
        _ => return ("400 Bad Request".into(), b"missing info_hash".to_vec()),
    };

    let port: u16 = params
        .get("port")
        .and_then(|v| std::str::from_utf8(v).ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let event = params
        .get("event")
        .and_then(|v| std::str::from_utf8(v).ok())
        .unwrap_or("");

    let peer_ip = match addr.ip() {
        std::net::IpAddr::V4(ip) => ip,
        std::net::IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .unwrap_or(Ipv4Addr::LOCALHOST),
    };

    let body = state.announce(info_hash, peer_ip, port, event);
    ("200 OK".into(), body)
}

fn parse_query(qs: &str) -> std::collections::HashMap<String, Vec<u8>> {
    let mut params = std::collections::HashMap::new();
    for part in qs.split('&') {
        if let Some((key, value)) = part.split_once('=') {
            let key = percent_decode(key.as_bytes());
            let value_bytes = percent_decode(value.as_bytes());
            params.insert(
                String::from_utf8_lossy(&key).to_string(),
                value_bytes,
            );
        }
    }
    params
}

fn percent_decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' && i + 2 < input.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&input[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
    out
}
