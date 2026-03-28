use std::{
    collections::{HashMap, hash_map::Entry},
    ffi::CStr,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use librqbit_dualstack_sockets::{BindDevice, UdpSocket};
use librtbit_core::{hash_id::Id20, spawn_utils::spawn_with_cancel};
use parking_lot::RwLock;
use rand::Rng;
use tokio_util::sync::CancellationToken;
use tracing::{debug, debug_span, trace, warn};

const ACTION_CONNECT: u32 = 0;
const ACTION_ANNOUNCE: u32 = 1;
// const ACTION_SCRAPE: u32 = 2;
const ACTION_ERROR: u32 = 3;

pub const EVENT_NONE: u32 = 0;
pub const EVENT_COMPLETED: u32 = 1;
pub const EVENT_STARTED: u32 = 2;
pub const EVENT_STOPPED: u32 = 3;

pub type ConnectionId = u64;
const CONNECTION_ID_MAGIC: ConnectionId = 0x41727101980;

pub type TransactionId = u32;

pub fn new_transaction_id() -> TransactionId {
    rand::rng().random()
}

#[derive(Debug)]
pub struct AnnounceFields {
    pub info_hash: Id20,
    pub peer_id: Id20,
    pub downloaded: u64,
    pub left: u64,
    pub uploaded: u64,
    pub event: u32,
    pub key: u32,
    pub port: u16,
}

#[derive(Debug)]
pub enum Request {
    Connect,
    Announce(ConnectionId, AnnounceFields),
}

impl Request {
    pub fn serialize(
        &self,
        transaction_id: TransactionId,
        buf: &mut [u8],
    ) -> anyhow::Result<usize> {
        struct W<'a> {
            buf: &'a mut [u8],
            offset: usize,
        }
        impl W<'_> {
            fn extend_from_slice(&mut self, s: &[u8]) -> anyhow::Result<()> {
                if self.buf.len() < self.offset + s.len() {
                    bail!("not enough space in buffer")
                }
                self.buf[self.offset..self.offset + s.len()].copy_from_slice(s);
                self.offset += s.len();
                Ok(())
            }
        }

        let mut w = W { buf, offset: 0 };

        match self {
            Request::Connect => {
                w.extend_from_slice(&CONNECTION_ID_MAGIC.to_be_bytes())?;
                w.extend_from_slice(&ACTION_CONNECT.to_be_bytes())?;
                w.extend_from_slice(&transaction_id.to_be_bytes())?;
            }
            Request::Announce(connection_id, fields) => {
                w.extend_from_slice(&connection_id.to_be_bytes())?;
                w.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes())?;
                w.extend_from_slice(&transaction_id.to_be_bytes())?;
                w.extend_from_slice(&fields.info_hash.0)?;
                w.extend_from_slice(&fields.peer_id.0)?;
                w.extend_from_slice(&fields.downloaded.to_be_bytes())?;
                w.extend_from_slice(&fields.left.to_be_bytes())?;
                w.extend_from_slice(&fields.uploaded.to_be_bytes())?;
                w.extend_from_slice(&fields.event.to_be_bytes())?;
                w.extend_from_slice(&0u32.to_be_bytes())?; // ip address 0
                w.extend_from_slice(&fields.key.to_be_bytes())?;
                w.extend_from_slice(&(-1i32).to_be_bytes())?; // num want -1
                w.extend_from_slice(&fields.port.to_be_bytes())?;
            }
        }
        Ok(w.offset)
    }
}

#[derive(Debug)]
pub struct AnnounceResponse {
    pub interval: u32,
    #[allow(dead_code)]
    pub leechers: u32,
    #[allow(dead_code)]
    pub seeders: u32,
    pub addrs: Vec<SocketAddr>,
}

#[derive(Debug)]
pub enum Response {
    Connect(ConnectionId),
    Announce(AnnounceResponse),
    #[allow(dead_code)]
    Error(String),
    Unknown,
}

fn split_slice(s: &[u8], first_len: usize) -> Option<(&[u8], &[u8])> {
    if s.len() < first_len {
        return None;
    }
    Some(s.split_at(first_len))
}

fn s_to_arr<const T: usize>(buf: &[u8]) -> [u8; T] {
    let mut arr = [0u8; T];
    arr.copy_from_slice(buf);
    arr
}

trait ParseNum: Sized {
    fn parse_num(buf: &[u8]) -> anyhow::Result<(Self, &[u8])>;
}

macro_rules! parse_impl {
    ($ty:tt, $size:expr) => {
        impl ParseNum for $ty {
            fn parse_num(buf: &[u8]) -> anyhow::Result<($ty, &[u8])> {
                let (bytes, rest) =
                    split_slice(buf, $size).with_context(|| format!("expected {} bytes", $size))?;
                let num = $ty::from_be_bytes(s_to_arr(bytes));
                Ok((num, rest))
            }
        }
    };
}

parse_impl!(u32, 4);
parse_impl!(u64, 8);
parse_impl!(u128, 16);
parse_impl!(u16, 2);
parse_impl!(i32, 4);
parse_impl!(i64, 8);
parse_impl!(i16, 2);

impl Response {
    pub fn parse(buf: &[u8], is_ipv6: bool) -> anyhow::Result<(TransactionId, Self)> {
        let (action, buf) = u32::parse_num(buf).context("can't parse action")?;
        let (tid, buf) = u32::parse_num(buf).context("can't parse transaction id")?;

        let response = match Self::parse_response(action, is_ipv6, buf) {
            Ok(r) => r,
            Err(e) => {
                debug!("error parsing: {e:#}");
                Response::Unknown
            }
        };

        Ok((tid, response))
    }

    fn parse_response(action: u32, is_ipv6: bool, mut buf: &[u8]) -> anyhow::Result<Self> {
        let response = match action {
            ACTION_CONNECT => {
                let (connection_id, b) =
                    u64::parse_num(buf).context("can't parse connection id")?;
                buf = b;
                Response::Connect(connection_id)
            }
            ACTION_ANNOUNCE => {
                let (interval, b) = u32::parse_num(buf).context("can't parse interval")?;
                let (leechers, b) = u32::parse_num(b).context("can't parse leechers")?;
                let (seeders, mut b) = u32::parse_num(b).context("can't parse seeders")?;
                let mut addrs = Vec::new();
                while !b.is_empty() {
                    let (addr, b2) = if is_ipv6 {
                        let (ip, b2) = u128::parse_num(b)?;
                        (IpAddr::V6(Ipv6Addr::from(ip)), b2)
                    } else {
                        let (ip, b2) = u32::parse_num(b)?;
                        (IpAddr::V4(Ipv4Addr::from(ip)), b2)
                    };
                    b = b2;

                    let (port, b2) = u16::parse_num(b)?;
                    b = b2;
                    addrs.push(SocketAddr::new(addr, port));
                }
                buf = b;
                Response::Announce(AnnounceResponse {
                    interval,
                    leechers,
                    seeders,
                    addrs,
                })
            }
            ACTION_ERROR => {
                let msg = CStr::from_bytes_with_nul(buf)
                    .ok()
                    .and_then(|s| s.to_str().ok())
                    .or_else(|| std::str::from_utf8(buf).ok())
                    .unwrap_or("<invalid UTF-8>")
                    .to_owned();
                return Ok(Response::Error(msg));
            }
            _ => bail!("unsupported action {action}"),
        };

        if !buf.is_empty() {
            bail!(
                "parsed {response:?} so far, but got {} remaining bytes",
                buf.len()
            );
        }

        Ok(response)
    }
}

struct ConnectionIdMeta {
    id: ConnectionId,
    created: Instant,
}

#[derive(Default)]
struct ClientLocked {
    connections: HashMap<SocketAddr, ConnectionIdMeta>,
    transactions: HashMap<TransactionId, tokio::sync::oneshot::Sender<Response>>,
}

struct ClientShared {
    sock: UdpSocket,
    locked: RwLock<ClientLocked>,
}

#[derive(Clone)]
pub struct UdpTrackerClient {
    state: Arc<ClientShared>,
}

struct TransactionIdGuard<'a> {
    tid: TransactionId,
    state: &'a ClientShared,
}

impl Drop for TransactionIdGuard<'_> {
    fn drop(&mut self) {
        let mut g = self.state.locked.write();
        g.transactions.remove(&self.tid);
    }
}

impl UdpTrackerClient {
    pub async fn new(
        cancel_token: CancellationToken,
        bind_device: Option<&BindDevice>,
    ) -> anyhow::Result<Self> {
        let addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0);
        let sock = UdpSocket::bind_udp(
            addr,
            librqbit_dualstack_sockets::BindOpts {
                device: bind_device,
                ..Default::default()
            },
        )
        .with_context(|| format!("error creating UDP socket at {addr}"))?;

        let client = Self {
            state: Arc::new(ClientShared {
                sock,
                locked: RwLock::new(Default::default()),
            }),
        };

        spawn_with_cancel(debug_span!("udp_tracker"), "udp_tracker", cancel_token, {
            let client = client.clone();
            async move { client.run().await }
        });

        Ok(client)
    }

    async fn run(self) -> anyhow::Result<()> {
        let mut buf = [0u8; 16384];
        loop {
            let (len, addr) = match self.state.sock.recv_from(&mut buf).await {
                Ok(r) => r,
                Err(e) => {
                    warn!("error in UdpSocket::recv_from: {e:#}");
                    continue;
                }
            };

            let (tid, response) = match Response::parse(&buf[..len], addr.is_ipv6()) {
                Ok(r) => r,
                Err(e) => {
                    debug!(?addr, "error parsing UDP response: {e:#}");
                    continue;
                }
            };

            trace!(?tid, ?response, ?addr, "received");

            let t = self.state.locked.write().transactions.remove(&tid);
            match t {
                Some(tx) => match tx.send(response) {
                    Ok(_) => {}
                    Err(_) => {
                        debug!(tid, "reader dead");
                    }
                },
                None => {
                    debug!(tid, "nowhere to send response");
                }
            };
        }
    }

    async fn get_connection_id(&self, addr: SocketAddr) -> anyhow::Result<ConnectionId> {
        if let Some(m) = self.state.locked.read().connections.get(&addr)
            && m.created.elapsed() < Duration::from_secs(60)
        {
            return Ok(m.id);
        }

        let response = self.request(addr, Request::Connect).await?;
        match response {
            Response::Connect(connection_id) => {
                self.state.locked.write().connections.insert(
                    addr,
                    ConnectionIdMeta {
                        id: connection_id,
                        created: Instant::now(),
                    },
                );
                Ok(connection_id)
            }
            _ => anyhow::bail!("expected connect response"),
        }
    }

    async fn request(&self, addr: SocketAddr, request: Request) -> anyhow::Result<Response> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tid_g = self.reserve_transaction_id(tx)?;

        let mut write_buf = [0u8; 1024];
        let len = request.serialize(tid_g.tid, &mut write_buf)?;
        self.state
            .sock
            .send_to(&write_buf[..len], addr)
            .await
            .with_context(|| format!("error sending to {addr:?}"))?;

        let response = tokio::time::timeout(Duration::from_secs(10), rx)
            .await
            .context("timeout connecting")?
            .context("sender dead")?;
        match &response {
            Response::Error(e) => {
                anyhow::bail!("remote errored: {e}")
            }
            Response::Unknown => {
                anyhow::bail!("remote replied with something we could not parse")
            }
            _ => {}
        }
        Ok(response)
    }

    fn reserve_transaction_id(
        &self,
        tx: tokio::sync::oneshot::Sender<Response>,
    ) -> anyhow::Result<TransactionIdGuard<'_>> {
        let mut g = self.state.locked.write();
        for _ in 0..10 {
            let t = new_transaction_id();
            match g.transactions.entry(t) {
                Entry::Occupied(_) => continue,
                Entry::Vacant(vac) => {
                    vac.insert(tx);
                    return Ok(TransactionIdGuard {
                        tid: t,
                        state: &self.state,
                    });
                }
            }
        }
        bail!("cant generate transaction id")
    }

    pub async fn announce(
        &self,
        tracker: SocketAddr,
        fields: AnnounceFields,
    ) -> anyhow::Result<AnnounceResponse> {
        let connection_id = self.get_connection_id(tracker).await?;
        let request = Request::Announce(connection_id, fields);
        let response = self.request(tracker, request).await?;
        match response {
            Response::Announce(r) => Ok(r),
            other => bail!("unexpected response {other:?}, expected announce"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Write, str::FromStr};

    use librtbit_core::{hash_id::Id20, peer_id::generate_peer_id};

    use crate::tracker_comms_udp::{
        AnnounceFields, CONNECTION_ID_MAGIC, ConnectionId, EVENT_NONE, EVENT_STARTED, Request,
        Response, TransactionId, new_transaction_id,
    };

    use super::{ACTION_ANNOUNCE, ACTION_CONNECT, ACTION_ERROR};

    #[test]
    fn test_parse_announce() {
        let b = include_bytes!("../resources/test/udp-tracker-announce-response.bin");
        let (tid, response) = Response::parse(b, false).unwrap();
        dbg!(tid, response);
    }

    #[test]
    fn test_udp_connect_request_serialize() {
        // BEP 15: connect request is 16 bytes:
        //   0-7:  magic connection_id (0x41727101980)
        //   8-11: action (0 = connect)
        //   12-15: transaction_id
        let tid: TransactionId = 0xDEADBEEF;
        let mut buf = [0u8; 64];
        let len = Request::Connect.serialize(tid, &mut buf).unwrap();
        assert_eq!(len, 16);

        // Check magic connection_id.
        let cid = u64::from_be_bytes(buf[0..8].try_into().unwrap());
        assert_eq!(cid, CONNECTION_ID_MAGIC);

        // Check action = 0 (connect).
        let action = u32::from_be_bytes(buf[8..12].try_into().unwrap());
        assert_eq!(action, ACTION_CONNECT);

        // Check transaction_id.
        let parsed_tid = u32::from_be_bytes(buf[12..16].try_into().unwrap());
        assert_eq!(parsed_tid, tid);
    }

    #[test]
    fn test_udp_connect_response_parse() {
        // Build a connect response: action=0, tid, connection_id
        let tid: TransactionId = 42;
        let connection_id: ConnectionId = 0x1234567890ABCDEF;

        let mut buf = Vec::new();
        buf.extend_from_slice(&ACTION_CONNECT.to_be_bytes());
        buf.extend_from_slice(&tid.to_be_bytes());
        buf.extend_from_slice(&connection_id.to_be_bytes());

        let (parsed_tid, response) = Response::parse(&buf, false).unwrap();
        assert_eq!(parsed_tid, tid);
        match response {
            Response::Connect(cid) => assert_eq!(cid, connection_id),
            other => panic!("expected Connect, got {:?}", other),
        }
    }

    #[test]
    fn test_udp_announce_request_serialize() {
        // BEP 15: announce request is 98 bytes:
        //   0-7:   connection_id
        //   8-11:  action (1 = announce)
        //   12-15: transaction_id
        //   16-35: info_hash (20 bytes)
        //   36-55: peer_id (20 bytes)
        //   56-63: downloaded
        //   64-71: left
        //   72-79: uploaded
        //   80-83: event
        //   84-87: IP address (0)
        //   88-91: key
        //   92-95: num_want (-1)
        //   96-97: port
        let connection_id: ConnectionId = 0xABCDEF0123456789;
        let tid: TransactionId = 0x11223344;
        let info_hash = Id20::new([0xAA; 20]);
        let peer_id = Id20::new([0xBB; 20]);

        let fields = AnnounceFields {
            info_hash,
            peer_id,
            downloaded: 1000,
            left: 50000,
            uploaded: 2000,
            event: EVENT_STARTED,
            key: 0x55667788,
            port: 6881,
        };

        let request = Request::Announce(connection_id, fields);
        let mut buf = [0u8; 128];
        let len = request.serialize(tid, &mut buf).unwrap();
        assert_eq!(len, 98);

        // Verify connection_id.
        let cid = u64::from_be_bytes(buf[0..8].try_into().unwrap());
        assert_eq!(cid, connection_id);

        // Verify action = 1 (announce).
        let action = u32::from_be_bytes(buf[8..12].try_into().unwrap());
        assert_eq!(action, ACTION_ANNOUNCE);

        // Verify transaction_id.
        let parsed_tid = u32::from_be_bytes(buf[12..16].try_into().unwrap());
        assert_eq!(parsed_tid, tid);

        // Verify info_hash.
        assert_eq!(&buf[16..36], &[0xAA; 20]);

        // Verify peer_id.
        assert_eq!(&buf[36..56], &[0xBB; 20]);

        // Verify downloaded.
        let downloaded = u64::from_be_bytes(buf[56..64].try_into().unwrap());
        assert_eq!(downloaded, 1000);

        // Verify left.
        let left = u64::from_be_bytes(buf[64..72].try_into().unwrap());
        assert_eq!(left, 50000);

        // Verify uploaded.
        let uploaded = u64::from_be_bytes(buf[72..80].try_into().unwrap());
        assert_eq!(uploaded, 2000);

        // Verify event.
        let event = u32::from_be_bytes(buf[80..84].try_into().unwrap());
        assert_eq!(event, EVENT_STARTED);

        // Verify IP = 0.
        let ip = u32::from_be_bytes(buf[84..88].try_into().unwrap());
        assert_eq!(ip, 0);

        // Verify key.
        let key = u32::from_be_bytes(buf[88..92].try_into().unwrap());
        assert_eq!(key, 0x55667788);

        // Verify num_want = -1.
        let num_want = i32::from_be_bytes(buf[92..96].try_into().unwrap());
        assert_eq!(num_want, -1);

        // Verify port.
        let port = u16::from_be_bytes(buf[96..98].try_into().unwrap());
        assert_eq!(port, 6881);
    }

    #[test]
    fn test_udp_announce_response_parse() {
        // Build an announce response with 2 IPv4 peers.
        let tid: TransactionId = 99;
        let interval: u32 = 1800;
        let leechers: u32 = 10;
        let seeders: u32 = 50;

        let mut buf = Vec::new();
        buf.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes());
        buf.extend_from_slice(&tid.to_be_bytes());
        buf.extend_from_slice(&interval.to_be_bytes());
        buf.extend_from_slice(&leechers.to_be_bytes());
        buf.extend_from_slice(&seeders.to_be_bytes());

        // Peer 1: 192.168.1.1:6881
        buf.extend_from_slice(&[192, 168, 1, 1]);
        buf.extend_from_slice(&6881u16.to_be_bytes());

        // Peer 2: 10.0.0.5:51413
        buf.extend_from_slice(&[10, 0, 0, 5]);
        buf.extend_from_slice(&51413u16.to_be_bytes());

        let (parsed_tid, response) = Response::parse(&buf, false).unwrap();
        assert_eq!(parsed_tid, tid);
        match response {
            Response::Announce(ann) => {
                assert_eq!(ann.interval, interval);
                assert_eq!(ann.leechers, leechers);
                assert_eq!(ann.seeders, seeders);
                assert_eq!(ann.addrs.len(), 2);
                assert_eq!(ann.addrs[0], "192.168.1.1:6881".parse().unwrap());
                assert_eq!(ann.addrs[1], "10.0.0.5:51413".parse().unwrap());
            }
            other => panic!("expected Announce, got {:?}", other),
        }
    }

    #[test]
    fn test_udp_announce_response_parse_ipv6() {
        // Build an announce response with 1 IPv6 peer.
        let tid: TransactionId = 200;
        let interval: u32 = 900;
        let leechers: u32 = 3;
        let seeders: u32 = 7;

        let mut buf = Vec::new();
        buf.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes());
        buf.extend_from_slice(&tid.to_be_bytes());
        buf.extend_from_slice(&interval.to_be_bytes());
        buf.extend_from_slice(&leechers.to_be_bytes());
        buf.extend_from_slice(&seeders.to_be_bytes());

        // IPv6 peer: 2001:db8::1 port 8080
        let ipv6 = std::net::Ipv6Addr::from_str("2001:db8::1").unwrap();
        buf.extend_from_slice(&u128::from(ipv6).to_be_bytes());
        buf.extend_from_slice(&8080u16.to_be_bytes());

        let (parsed_tid, response) = Response::parse(&buf, true).unwrap();
        assert_eq!(parsed_tid, tid);
        match response {
            Response::Announce(ann) => {
                assert_eq!(ann.interval, interval);
                assert_eq!(ann.addrs.len(), 1);
                assert_eq!(ann.addrs[0], "[2001:db8::1]:8080".parse().unwrap());
            }
            other => panic!("expected Announce, got {:?}", other),
        }
    }

    #[test]
    fn test_udp_error_response_parse() {
        // Build an error response: action=3, tid, then error message as UTF-8.
        let tid: TransactionId = 777;
        let error_msg = "Connection ID mismatched";

        let mut buf = Vec::new();
        buf.extend_from_slice(&ACTION_ERROR.to_be_bytes());
        buf.extend_from_slice(&tid.to_be_bytes());
        buf.extend_from_slice(error_msg.as_bytes());

        let (parsed_tid, response) = Response::parse(&buf, false).unwrap();
        assert_eq!(parsed_tid, tid);
        match response {
            Response::Error(msg) => {
                assert_eq!(msg, error_msg);
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[test]
    fn test_udp_error_response_parse_with_nul() {
        // Error message as a C string (null-terminated).
        let tid: TransactionId = 888;

        let mut buf = Vec::new();
        buf.extend_from_slice(&ACTION_ERROR.to_be_bytes());
        buf.extend_from_slice(&tid.to_be_bytes());
        buf.extend_from_slice(b"tracker error\0");

        let (parsed_tid, response) = Response::parse(&buf, false).unwrap();
        assert_eq!(parsed_tid, tid);
        match response {
            Response::Error(msg) => {
                assert_eq!(msg, "tracker error");
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[test]
    fn test_udp_announce_response_no_peers() {
        // Announce response with zero peers.
        let tid: TransactionId = 300;

        let mut buf = Vec::new();
        buf.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes());
        buf.extend_from_slice(&tid.to_be_bytes());
        buf.extend_from_slice(&1800u32.to_be_bytes()); // interval
        buf.extend_from_slice(&0u32.to_be_bytes()); // leechers
        buf.extend_from_slice(&0u32.to_be_bytes()); // seeders

        let (parsed_tid, response) = Response::parse(&buf, false).unwrap();
        assert_eq!(parsed_tid, tid);
        match response {
            Response::Announce(ann) => {
                assert_eq!(ann.interval, 1800);
                assert!(ann.addrs.is_empty());
            }
            other => panic!("expected Announce, got {:?}", other),
        }
    }

    #[test]
    fn test_udp_malformed_response_too_short() {
        // A buffer that is too short to even parse action + tid.
        let buf = [0u8; 3];
        let result = Response::parse(&buf, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_udp_malformed_response_truncated_connect() {
        // A connect response that is too short (missing connection_id).
        let tid: TransactionId = 50;
        let mut buf = Vec::new();
        buf.extend_from_slice(&ACTION_CONNECT.to_be_bytes());
        buf.extend_from_slice(&tid.to_be_bytes());
        // Missing 8 bytes for connection_id.

        let (parsed_tid, response) = Response::parse(&buf, false).unwrap();
        assert_eq!(parsed_tid, tid);
        // Should parse as Unknown since the inner parse_response fails.
        assert!(matches!(response, Response::Unknown));
    }

    #[test]
    fn test_udp_malformed_response_truncated_announce() {
        // Announce response that is too short (only has interval, no leechers/seeders).
        let tid: TransactionId = 51;
        let mut buf = Vec::new();
        buf.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes());
        buf.extend_from_slice(&tid.to_be_bytes());
        buf.extend_from_slice(&1800u32.to_be_bytes()); // interval only
        // Missing leechers and seeders.

        let (parsed_tid, response) = Response::parse(&buf, false).unwrap();
        assert_eq!(parsed_tid, tid);
        assert!(matches!(response, Response::Unknown));
    }

    #[test]
    fn test_udp_connect_request_buffer_too_small() {
        // Connect request needs 16 bytes; provide less.
        let mut buf = [0u8; 10];
        let result = Request::Connect.serialize(1, &mut buf);
        assert!(result.is_err());
    }

    #[test]
    fn test_udp_announce_request_buffer_too_small() {
        let fields = AnnounceFields {
            info_hash: Id20::new([0; 20]),
            peer_id: Id20::new([0; 20]),
            downloaded: 0,
            left: 0,
            uploaded: 0,
            event: EVENT_NONE,
            key: 0,
            port: 0,
        };
        // Announce request needs 98 bytes; provide less.
        let mut buf = [0u8; 50];
        let result = Request::Announce(1, fields).serialize(1, &mut buf);
        assert!(result.is_err());
    }

    #[test]
    fn test_udp_announce_response_parse_multiple_ipv4_peers() {
        // Build an announce response with 5 IPv4 peers to test iteration.
        let tid: TransactionId = 400;

        let mut buf = Vec::new();
        buf.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes());
        buf.extend_from_slice(&tid.to_be_bytes());
        buf.extend_from_slice(&600u32.to_be_bytes()); // interval
        buf.extend_from_slice(&100u32.to_be_bytes()); // leechers
        buf.extend_from_slice(&200u32.to_be_bytes()); // seeders

        let expected_peers = vec![
            "1.1.1.1:80",
            "2.2.2.2:443",
            "3.3.3.3:6881",
            "4.4.4.4:51413",
            "5.5.5.5:8080",
        ];

        for peer_str in &expected_peers {
            let addr: std::net::SocketAddrV4 = peer_str.parse().unwrap();
            buf.extend_from_slice(&addr.ip().octets());
            buf.extend_from_slice(&addr.port().to_be_bytes());
        }

        let (parsed_tid, response) = Response::parse(&buf, false).unwrap();
        assert_eq!(parsed_tid, tid);
        match response {
            Response::Announce(ann) => {
                assert_eq!(ann.interval, 600);
                assert_eq!(ann.leechers, 100);
                assert_eq!(ann.seeders, 200);
                assert_eq!(ann.addrs.len(), 5);
                for (i, peer_str) in expected_peers.iter().enumerate() {
                    assert_eq!(
                        ann.addrs[i],
                        peer_str.parse::<std::net::SocketAddr>().unwrap()
                    );
                }
            }
            other => panic!("expected Announce, got {:?}", other),
        }
    }

    #[test]
    fn test_udp_announce_response_multiple_ipv6_peers() {
        let tid: TransactionId = 500;

        let mut buf = Vec::new();
        buf.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes());
        buf.extend_from_slice(&tid.to_be_bytes());
        buf.extend_from_slice(&300u32.to_be_bytes()); // interval
        buf.extend_from_slice(&5u32.to_be_bytes()); // leechers
        buf.extend_from_slice(&15u32.to_be_bytes()); // seeders

        let expected_peers = vec!["[::1]:6881", "[fe80::1]:8080", "[2001:db8::1]:443"];

        for peer_str in &expected_peers {
            let addr: std::net::SocketAddrV6 = peer_str.parse().unwrap();
            buf.extend_from_slice(&u128::from(*addr.ip()).to_be_bytes());
            buf.extend_from_slice(&addr.port().to_be_bytes());
        }

        let (parsed_tid, response) = Response::parse(&buf, true).unwrap();
        assert_eq!(parsed_tid, tid);
        match response {
            Response::Announce(ann) => {
                assert_eq!(ann.addrs.len(), 3);
                for (i, peer_str) in expected_peers.iter().enumerate() {
                    assert_eq!(
                        ann.addrs[i],
                        peer_str.parse::<std::net::SocketAddr>().unwrap()
                    );
                }
            }
            other => panic!("expected Announce, got {:?}", other),
        }
    }

    #[test]
    fn test_udp_connect_roundtrip() {
        // Serialize a connect request, then build a matching connect response,
        // and verify the transaction ID is consistent.
        let tid: TransactionId = 0xCAFEBABE;
        let mut write_buf = [0u8; 64];
        let len = Request::Connect.serialize(tid, &mut write_buf).unwrap();
        assert_eq!(len, 16);

        // Simulate a server response.
        let connection_id: ConnectionId = 0x123456789ABCDEF0;
        let mut response_buf = Vec::new();
        response_buf.extend_from_slice(&ACTION_CONNECT.to_be_bytes());
        response_buf.extend_from_slice(&tid.to_be_bytes());
        response_buf.extend_from_slice(&connection_id.to_be_bytes());

        let (parsed_tid, response) = Response::parse(&response_buf, false).unwrap();
        assert_eq!(parsed_tid, tid);
        match response {
            Response::Connect(cid) => assert_eq!(cid, connection_id),
            other => panic!("expected Connect, got {:?}", other),
        }
    }

    #[test]
    fn test_udp_unsupported_action() {
        // An action that is not connect/announce/error should yield Unknown.
        let tid: TransactionId = 999;
        let unsupported_action: u32 = 99;

        let mut buf = Vec::new();
        buf.extend_from_slice(&unsupported_action.to_be_bytes());
        buf.extend_from_slice(&tid.to_be_bytes());

        let (parsed_tid, response) = Response::parse(&buf, false).unwrap();
        assert_eq!(parsed_tid, tid);
        assert!(matches!(response, Response::Unknown));
    }

    #[ignore]
    #[tokio::test]
    async fn test_announce() {
        let sock = tokio::net::UdpSocket::bind("0.0.0.0:0").await.unwrap();
        sock.connect("opentor.net:6969").await.unwrap();

        let tid = new_transaction_id();
        let mut write_buf = [0u8; 16384];
        let mut read_buf = vec![0u8; 4096];

        let len = Request::Connect.serialize(tid, &mut write_buf).unwrap();

        sock.send(&write_buf[..len]).await.unwrap();

        let size = sock.recv(&mut read_buf).await.unwrap();

        let (rtid, response) = Response::parse(&read_buf[..size], false).unwrap();
        assert_eq!(tid, rtid);
        let connection_id = match response {
            Response::Connect(connection_id) => {
                dbg!(connection_id)
            }
            other => panic!("unexpected response {:?}", other),
        };

        let hash = Id20::from_str("775459190aa65566591634203f8d9f17d341f969").unwrap();

        let tid = new_transaction_id();
        let request = Request::Announce(
            connection_id,
            AnnounceFields {
                info_hash: hash,
                peer_id: generate_peer_id(b"-xx1234-"),
                downloaded: 0,
                left: 0,
                uploaded: 0,
                event: EVENT_NONE,
                key: 0, // whatever that is?
                port: 24563,
            },
        );
        let size = request.serialize(tid, &mut write_buf).unwrap();

        sock.send(&write_buf[..size]).await.unwrap();
        let size = sock.recv(&mut read_buf).await.unwrap();

        {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open("/tmp/proto.bin")
                .unwrap();
            f.write_all(&read_buf[..size]).unwrap();
        }

        dbg!(&read_buf[..size]);
        let (rtid, response) = Response::parse(&read_buf[..size], false).unwrap();
        assert_eq!(tid, rtid);
        match response {
            Response::Announce(r) => {
                dbg!(r);
            }
            other => panic!("unexpected response {:?}", other),
        }
    }
}
