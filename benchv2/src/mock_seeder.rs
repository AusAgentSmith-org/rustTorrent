use crate::bencode::{self, BValue};
use anyhow::Result;
use memmap2::Mmap;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

const PSTR: &[u8] = b"BitTorrent protocol";
const HANDSHAKE_LEN: usize = 68;
const MSG_UNCHOKE: u8 = 1;
const MSG_INTERESTED: u8 = 2;
const MSG_BITFIELD: u8 = 5;
const MSG_REQUEST: u8 = 6;
const MSG_PIECE: u8 = 7;

struct TorrentMeta {
    info_hash: [u8; 20],
    piece_length: u64,
    total_length: u64,
    num_pieces: usize,
    mmap: Mmap,
    data_path: PathBuf,
}

impl TorrentMeta {
    fn from_torrent_file(torrent_path: &Path, data_dir: &Path) -> Result<Self> {
        let raw = std::fs::read(torrent_path)?;
        let (bval, _) = bencode::decode(&raw)?;
        let dict = bencode::as_dict(&bval).ok_or_else(|| anyhow::anyhow!("not a dict"))?;
        let info_dict = bencode::dict_get(dict, "info")
            .and_then(bencode::as_dict)
            .ok_or_else(|| anyhow::anyhow!("no info dict"))?;

        let info_bval = BValue::Dict(info_dict.clone());
        let info_encoded = info_bval.encode();
        let hash = Sha1::digest(&info_encoded);
        let mut info_hash = [0u8; 20];
        info_hash.copy_from_slice(&hash);

        let name = bencode::dict_get(info_dict, "name")
            .and_then(bencode::as_bytes)
            .ok_or_else(|| anyhow::anyhow!("no name"))?;
        let piece_length = bencode::dict_get(info_dict, "piece length")
            .and_then(bencode::as_int)
            .ok_or_else(|| anyhow::anyhow!("no piece length"))? as u64;
        let total_length = bencode::dict_get(info_dict, "length")
            .and_then(bencode::as_int)
            .ok_or_else(|| anyhow::anyhow!("no length"))? as u64;

        let num_pieces = ((total_length + piece_length - 1) / piece_length) as usize;
        let file_name = String::from_utf8_lossy(name);
        let data_path = data_dir.join(file_name.as_ref());

        if !data_path.exists() {
            anyhow::bail!("data file missing: {}", data_path.display());
        }

        let file = std::fs::File::open(&data_path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        // Advise the OS we'll access sequentially — enables aggressive prefetch.
        mmap.advise(memmap2::Advice::Sequential)?;

        tracing::info!(
            "mmap'd {} ({:.1} GB)",
            data_path.display(),
            total_length as f64 / (1024.0 * 1024.0 * 1024.0)
        );

        Ok(Self {
            info_hash,
            piece_length,
            total_length,
            num_pieces,
            mmap,
            data_path,
        })
    }

    fn bitfield_bytes(&self) -> Vec<u8> {
        let n_bytes = (self.num_pieces + 7) / 8;
        let mut bf = vec![0u8; n_bytes];
        for i in 0..self.num_pieces {
            bf[i / 8] |= 1 << (7 - (i % 8));
        }
        bf
    }

    /// Slice a block directly from the memory-mapped file — zero syscalls.
    fn read_block(&self, piece: u32, offset: u32, length: u32) -> Result<&[u8]> {
        let file_offset = piece as u64 * self.piece_length + offset as u64;
        let end = file_offset + length as u64;
        if end > self.total_length {
            anyhow::bail!(
                "block out of range: offset {file_offset} + len {length} > {}",
                self.total_length
            );
        }
        Ok(&self.mmap[file_offset as usize..end as usize])
    }

    /// Advise the OS to drop page cache for this file's mmap region.
    /// Used between benchmark runs to ensure fair cold-cache conditions.
    /// Safety: MADV_DONTNEED on a file-backed mmap just evicts pages from cache;
    /// subsequent reads re-fault from disk. No data loss.
    fn drop_page_cache(&self) {
        unsafe {
            if let Err(e) = self.mmap.unchecked_advise(memmap2::UncheckedAdvice::DontNeed) {
                tracing::warn!("MADV_DONTNEED failed for {}: {e}", self.data_path.display());
            }
        }
    }
}

type TorrentMap = Arc<RwLock<HashMap<[u8; 20], Arc<TorrentMeta>>>>;

async fn handle_peer(
    mut stream: tokio::net::TcpStream,
    torrents: TorrentMap,
    peer_id: [u8; 20],
) {
    if let Err(e) = handle_peer_inner(&mut stream, &torrents, &peer_id).await {
        tracing::debug!("peer session ended: {e}");
    }
}

async fn handle_peer_inner(
    stream: &mut tokio::net::TcpStream,
    torrents: &TorrentMap,
    peer_id: &[u8; 20],
) -> Result<()> {
    // Read handshake
    let mut hs = vec![0u8; HANDSHAKE_LEN];
    tokio::time::timeout(std::time::Duration::from_secs(10), stream.read_exact(&mut hs)).await??;

    if hs[0] != 19 || &hs[1..20] != PSTR {
        anyhow::bail!("bad handshake pstr");
    }
    let mut info_hash = [0u8; 20];
    info_hash.copy_from_slice(&hs[28..48]);

    let torrent = {
        let map = torrents.read().await;
        map.get(&info_hash).cloned()
    };
    let torrent = match torrent {
        Some(t) => t,
        None => anyhow::bail!("unknown info_hash"),
    };

    // Send handshake
    let mut resp = vec![19u8];
    resp.extend_from_slice(PSTR);
    resp.extend_from_slice(&[0u8; 8]);
    resp.extend_from_slice(&info_hash);
    resp.extend_from_slice(peer_id);
    stream.write_all(&resp).await?;

    // Send bitfield
    let bf = torrent.bitfield_bytes();
    let mut msg = Vec::with_capacity(5 + bf.len());
    msg.extend_from_slice(&((1 + bf.len()) as u32).to_be_bytes());
    msg.push(MSG_BITFIELD);
    msg.extend_from_slice(&bf);
    stream.write_all(&msg).await?;

    // Send unchoke
    stream.write_all(&1u32.to_be_bytes()).await?;
    stream.write_all(&[MSG_UNCHOKE]).await?;

    // Message loop
    loop {
        let mut len_buf = [0u8; 4];
        tokio::time::timeout(
            std::time::Duration::from_secs(120),
            stream.read_exact(&mut len_buf),
        )
        .await??;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len == 0 {
            continue; // keepalive
        }
        let mut msg_buf = vec![0u8; len];
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            stream.read_exact(&mut msg_buf),
        )
        .await??;

        match msg_buf[0] {
            MSG_REQUEST if msg_buf.len() >= 13 => {
                let piece = u32::from_be_bytes(msg_buf[1..5].try_into()?);
                let offset = u32::from_be_bytes(msg_buf[5..9].try_into()?);
                let block_len = u32::from_be_bytes(msg_buf[9..13].try_into()?);

                // Slice directly from mmap — no syscalls, no allocation
                let data = torrent.read_block(piece, offset, block_len)?;

                // Send piece message
                let total_len = 9 + data.len();
                let mut header = [0u8; 13];
                header[0..4].copy_from_slice(&(total_len as u32).to_be_bytes());
                header[4] = MSG_PIECE;
                header[5..9].copy_from_slice(&piece.to_be_bytes());
                header[9..13].copy_from_slice(&offset.to_be_bytes());
                stream.write_all(&header).await?;
                stream.write_all(data).await?;
            }
            MSG_INTERESTED => {
                // Re-unchoke
                stream.write_all(&1u32.to_be_bytes()).await?;
                stream.write_all(&[MSG_UNCHOKE]).await?;
            }
            _ => {} // ignore other messages
        }
    }
}

async fn load_torrents(data_dir: &Path, torrent_dir: &Path) -> HashMap<[u8; 20], Arc<TorrentMeta>> {
    let mut map = HashMap::new();
    let Ok(mut entries) = tokio::fs::read_dir(torrent_dir).await else {
        return map;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "torrent") {
            continue;
        }
        match TorrentMeta::from_torrent_file(&path, data_dir) {
            Ok(meta) => {
                tracing::info!(
                    "Loaded: {} ({} MB, {} pieces)",
                    path.display(),
                    meta.total_length / (1024 * 1024),
                    meta.num_pieces
                );
                map.insert(meta.info_hash, Arc::new(meta));
            }
            Err(e) => tracing::debug!("Skip {}: {e}", path.display()),
        }
    }
    map
}

async fn announce_all(
    torrents: &HashMap<[u8; 20], Arc<TorrentMeta>>,
    tracker_url: &str,
    num_peers: usize,
    base_port: u16,
) {
    let client = reqwest::Client::new();
    let mut count = 0u32;
    for meta in torrents.values() {
        for i in 0..num_peers {
            let port = base_port + i as u16;
            let peer_id = format!("-MS0001-{:012}", i);
            let ih = percent_encoding::percent_encode(
                &meta.info_hash,
                percent_encoding::NON_ALPHANUMERIC,
            );
            let pid = percent_encoding::percent_encode(
                peer_id.as_bytes(),
                percent_encoding::NON_ALPHANUMERIC,
            );
            let url = format!(
                "{tracker_url}?info_hash={ih}&peer_id={pid}&port={port}&uploaded=0&downloaded=0&left=0&compact=1&event=started"
            );
            match client.get(&url).send().await {
                Ok(_) => count += 1,
                Err(e) => {
                    if count == 0 {
                        tracing::warn!("Announce failed: {e}");
                    }
                }
            }
        }
    }
    tracing::info!("Announced {count} (torrent, peer) pairs");
}

/// Reload torrents from disk, merge into the live map, and re-announce.
/// Returns the new total count.
async fn do_reload(
    torrents: &TorrentMap,
    data_dir: &Path,
    torrent_dir: &Path,
    tracker_url: &str,
    num_peers: usize,
    base_port: u16,
) -> usize {
    let loaded = load_torrents(data_dir, torrent_dir).await;
    let mut map = torrents.write().await;
    let before = map.len();
    for (k, v) in loaded {
        map.entry(k).or_insert(v);
    }
    let after = map.len();
    if after > before {
        tracing::info!("Reload: {} -> {} torrent(s)", before, after);
    }
    // Always re-announce — peers expire on the tracker after 120s
    tracing::info!("Re-announcing {} torrent(s) to tracker", after);
    announce_all(&map, tracker_url, num_peers, base_port).await;
    after
}

/// HTTP control server: /health, /status, /reload
async fn run_control_server(
    health_port: u16,
    torrents: TorrentMap,
    data_dir: PathBuf,
    torrent_dir: PathBuf,
    tracker_url: String,
    num_peers: usize,
    base_port: u16,
) {
    let listener = TcpListener::bind(("0.0.0.0", health_port)).await.unwrap();
    tracing::info!("Control server on port {health_port} (/health, /status, /reload)");

    loop {
        let Ok((mut stream, _)) = listener.accept().await else { continue };
        let torrents = torrents.clone();
        let data_dir = data_dir.clone();
        let torrent_dir = torrent_dir.clone();
        let tracker_url = tracker_url.clone();

        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let n = match stream.read(&mut buf).await {
                Ok(n) => n,
                Err(_) => return,
            };
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req.split_whitespace().nth(1).unwrap_or("/");

            let response = match path {
                "/health" => {
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nOK".to_string()
                }
                "/status" => {
                    let count = torrents.read().await.len();
                    let body = format!("{{\"torrents\":{count}}}");
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                }
                "/reload" => {
                    let count = do_reload(
                        &torrents, &data_dir, &torrent_dir,
                        &tracker_url, num_peers, base_port,
                    ).await;
                    let body = format!("{{\"torrents\":{count}}}");
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                }
                "/drop-cache" => {
                    // Advise DONTNEED on all mmap'd files to evict them from page cache.
                    // This ensures fair cold-cache conditions between benchmark runs.
                    let map = torrents.read().await;
                    let mut dropped = 0usize;
                    let mut total_bytes = 0u64;
                    for meta in map.values() {
                        meta.drop_page_cache();
                        total_bytes += meta.total_length;
                        dropped += 1;
                    }
                    let gb = total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                    tracing::info!(
                        "Dropped page cache for {dropped} file(s), {gb:.1} GB"
                    );
                    let body = format!(
                        "{{\"dropped\":{dropped},\"total_gb\":{gb:.1}}}"
                    );
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                }
                _ => {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_string()
                }
            };
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

pub async fn run(
    num_peers: usize,
    base_port: u16,
    tracker_url: String,
    data_dir: PathBuf,
    torrent_dir: PathBuf,
    health_port: u16,
) -> Result<()> {
    let torrents: TorrentMap = Arc::new(RwLock::new(HashMap::new()));

    // Start control server (health/status/reload)
    tokio::spawn(run_control_server(
        health_port,
        torrents.clone(),
        data_dir.clone(),
        torrent_dir.clone(),
        tracker_url.clone(),
        num_peers,
        base_port,
    ));

    // Wait for torrents to appear
    loop {
        let loaded = load_torrents(&data_dir, &torrent_dir).await;
        if !loaded.is_empty() {
            tracing::info!("Loaded {} torrent(s)", loaded.len());
            *torrents.write().await = loaded;
            break;
        }
        tracing::info!("No torrents yet, waiting...");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    // Start peer listeners
    let mut listeners = Vec::new();
    for i in 0..num_peers {
        let port = base_port + i as u16;
        let listener = TcpListener::bind(("0.0.0.0", port)).await?;
        listeners.push(listener);
    }
    tracing::info!(
        "Started {num_peers} virtual peers on ports {base_port}-{}",
        base_port + num_peers as u16 - 1
    );

    // Initial announce
    {
        let map = torrents.read().await;
        announce_all(&map, &tracker_url, num_peers, base_port).await;
    }

    // Spawn accept loops for each peer port
    for (i, listener) in listeners.into_iter().enumerate() {
        let torrents = torrents.clone();
        let mut peer_id = [0u8; 20];
        let id_str = format!("-MS0001-{:012}", i);
        peer_id[..20].copy_from_slice(&id_str.as_bytes()[..20]);

        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let torrents = torrents.clone();
                        tokio::spawn(handle_peer(stream, torrents, peer_id));
                    }
                    Err(e) => tracing::debug!("accept error port {}: {e}", base_port + i as u16),
                }
            }
        });
    }

    // Periodic re-announce and torrent re-scan
    let torrents_clone = torrents.clone();
    let tracker_url_clone = tracker_url.clone();
    let data_dir_clone = data_dir.clone();
    let torrent_dir_clone = torrent_dir.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(25)).await;
            do_reload(
                &torrents_clone, &data_dir_clone, &torrent_dir_clone,
                &tracker_url_clone, num_peers, base_port,
            ).await;
        }
    });

    // Run forever
    std::future::pending::<()>().await;
    Ok(())
}
