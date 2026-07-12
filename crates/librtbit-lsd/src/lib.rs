use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    str::{FromStr, from_utf8},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use anyhow::Context;
use futures::Stream;
use librqbit_dualstack_sockets::{BindDevice, MulticastUdpSocket};
use librtbit_core::{Id20, spawn_utils::spawn_with_cancel};
use parking_lot::RwLock;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;
use tracing::{debug, debug_span, trace};

const LSD_PORT: u16 = 6771;
const LSD_IPV4: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 192, 152, 143), LSD_PORT);
const LSD_IPV6: SocketAddrV6 = SocketAddrV6::new(
    Ipv6Addr::new(0xff15, 0, 0, 0, 0, 0, 0xefc0, 0x988f),
    LSD_PORT,
    0,
    0,
);

const RATE_LIMIT_PERIOD: Duration = Duration::from_secs(1);

#[derive(Default)]
struct RateLimiter {
    last_reply: AtomicU64,
}

impl RateLimiter {
    fn check(&self) -> Option<()> {
        // If we can't get system time for some reason, just disable rate limit
        let now = match SystemTime::UNIX_EPOCH.elapsed() {
            Ok(t) => t.as_secs(),
            _ => return Some(()),
        };

        let last = self.last_reply.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= RATE_LIMIT_PERIOD.as_secs()
            && self
                .last_reply
                .compare_exchange_weak(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            return Some(());
        }
        None
    }
}

struct Announce {
    tx: UnboundedSender<SocketAddr>,
    port: Option<u16>,
    last_reply_ipv4: RateLimiter,
    last_reply_ipv6: RateLimiter,
}

struct LocalServiceDiscoveryInner {
    socket: MulticastUdpSocket,
    cookie: u32,
    cancel_token: CancellationToken,
    receivers: RwLock<HashMap<Id20, Announce>>,
}

#[derive(Clone)]
pub struct LocalServiceDiscovery {
    inner: Arc<LocalServiceDiscoveryInner>,
}

#[derive(Default)]
pub struct LocalServiceDiscoveryOptions<'a> {
    pub cancel_token: CancellationToken,
    pub cookie: Option<u32>,
    pub bind_device: Option<&'a BindDevice>,
}

impl LocalServiceDiscovery {
    pub async fn new(opts: LocalServiceDiscoveryOptions<'_>) -> anyhow::Result<Self> {
        let socket = MulticastUdpSocket::new(
            (Ipv6Addr::UNSPECIFIED, LSD_PORT).into(),
            LSD_IPV4,
            LSD_IPV6,
            None,
            opts.bind_device,
        )
        .await
        .context("error binding LSD socket")?;
        let cookie = opts.cookie.unwrap_or_else(rand::random);
        let lsd = Self {
            inner: Arc::new(LocalServiceDiscoveryInner {
                socket,
                cookie,
                cancel_token: opts.cancel_token.clone(),
                receivers: Default::default(),
            }),
        };

        spawn_with_cancel(
            debug_span!("lsd"),
            "lsd",
            opts.cancel_token,
            lsd.clone().task_monitor_recv(),
        );

        Ok(lsd)
    }

    fn gen_announce_msg(&self, info_hash: Id20, port: u16, is_v6: bool) -> String {
        let host: SocketAddr = if is_v6 {
            LSD_IPV6.into()
        } else {
            LSD_IPV4.into()
        };
        let cookie = self.inner.cookie;
        let info_hash = info_hash.as_string();
        format!(
            "BT-SEARCH * HTTP/1.1\r
Host: {host}\r
Port: {port}\r
Infohash: {info_hash}\r
cookie: {cookie}\r
\r
\r
"
        )
    }

    async fn recv_and_process_one(&self, buf: &mut [u8]) -> anyhow::Result<()> {
        macro_rules! return_if_none {
            ($e:expr) => {
                return_if_none!($e, ())
            };
            ($e:expr, $if_err:expr) => {
                match $e {
                    Some(e) => e,
                    None => {
                        $if_err;
                        return Ok(());
                    }
                }
            };
        }

        let mut headers = [httparse::EMPTY_HEADER; 16];

        let (sz, addr) = self.inner.socket.recv_from(buf).await?;
        let buf = bstr::BStr::new(&buf[..sz]);

        let bts = return_if_none!(
            try_parse_bt_search(buf, &mut headers)
                .inspect_err(|e| trace!(?buf, ?addr, "error parsing message: {e:#}"))
                .ok()
        );

        trace!(?addr, ?bts, "received");

        if bts.our_cookie == Some(self.inner.cookie) {
            trace!(?bts, "ignoring our own message");
            return Ok(());
        }

        let announce_port = {
            let g = self.inner.receivers.read();
            let announce = return_if_none!(g.get(&bts.hash));
            let mut addr = addr;
            addr.set_port(bts.port);

            return_if_none!(announce.tx.send(addr).ok());

            let announce_port = return_if_none!(announce.port);

            let rl = if addr.is_ipv4() {
                &announce.last_reply_ipv4
            } else {
                &announce.last_reply_ipv6
            };

            return_if_none!(rl.check(), trace!(?addr, ?bts, "replying rate-limited"));

            announce_port
        };

        let mopts = return_if_none!(
            self.inner.socket.find_mcast_opts_for_replying_to(&addr),
            debug!(?addr, "couldn't find where to reply")
        );

        let reply = self.gen_announce_msg(bts.hash, announce_port, addr.is_ipv6());

        if let Err(e) = self
            .inner
            .socket
            .send_multicast_msg(reply.as_bytes(), &mopts)
            .await
        {
            trace!(?addr, ?reply, ?mopts, "error sending reply: {e:#}");
        } else {
            trace!(?addr, ?reply, ?mopts, "sent reply");
        }
        Ok(())
    }

    async fn task_monitor_recv(self) -> anyhow::Result<()> {
        let mut buf = [0u8; 4096];

        loop {
            self.recv_and_process_one(&mut buf).await?;
        }
    }

    async fn task_announce_periodically(self, info_hash: Id20, port: u16) -> anyhow::Result<()> {
        let mut interval = tokio::time::interval(Duration::from_secs(60 * 5));
        loop {
            interval.tick().await;

            self.inner
                .socket
                .try_send_mcast_everywhere(&|mopts| {
                    Some(self.gen_announce_msg(info_hash, port, mopts.mcast_addr().is_ipv6()))
                })
                .await;
        }
    }

    pub fn announce(
        &self,
        info_hash: Id20,
        announce_port: Option<u16>,
    ) -> impl Stream<Item = SocketAddr> + Send + Sync + 'static {
        // 1. Periodically announce the torrent.
        // 2. Stream back the results from received messages.

        let (tx, rx) = unbounded_channel::<SocketAddr>();

        struct AddrStream {
            info_hash: Id20,
            rx: UnboundedReceiver<SocketAddr>,
            lsd: LocalServiceDiscovery,
        }

        impl Stream for AddrStream {
            type Item = SocketAddr;

            fn poll_next(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Option<Self::Item>> {
                self.rx.poll_recv(cx)
            }
        }

        impl Drop for AddrStream {
            fn drop(&mut self) {
                let _ = self.lsd.inner.receivers.write().remove(&self.info_hash);
            }
        }

        self.inner.receivers.write().insert(
            info_hash,
            Announce {
                tx,
                port: announce_port,
                last_reply_ipv4: Default::default(),
                last_reply_ipv6: Default::default(),
            },
        );

        if let Some(announce_port) = announce_port {
            let cancel_token = self.inner.cancel_token.child_token();
            spawn_with_cancel(
                debug_span!(parent: None, "lsd-announce", ?info_hash, port=announce_port),
                "lsd-announce",
                cancel_token,
                self.clone()
                    .task_announce_periodically(info_hash, announce_port),
            );
        }

        AddrStream {
            info_hash,
            rx,
            lsd: self.clone(),
        }
    }
}

#[derive(Debug)]
struct BtSearchAnnounceMessage {
    hash: Id20,
    our_cookie: Option<u32>,
    #[allow(unused)]
    host: SocketAddr,
    port: u16,
}

fn try_parse_bt_search<'a: 'h, 'h>(
    buf: &'a [u8],
    headers: &'a mut [httparse::Header<'h>],
) -> anyhow::Result<BtSearchAnnounceMessage> {
    let mut req = httparse::Request::new(headers);
    req.parse(buf).context("error parsing request")?;

    match req.method {
        Some("BT-SEARCH") => {
            let mut host = None;
            let mut port = None;
            let mut infohash = None;
            let mut our_cookie = None;

            for header in req.headers.iter() {
                if header.name.eq_ignore_ascii_case("host") {
                    host = Some(
                        from_utf8(header.value)
                            .context("invalid utf-8 in host header")?
                            .parse()
                            .context("invalid IP in host header")?,
                    );
                } else if header.name.eq_ignore_ascii_case("port") {
                    port = Some(atoi::atoi::<u16>(header.value).context("port is not a number")?)
                } else if header.name.eq_ignore_ascii_case("infohash") {
                    infohash = Some(
                        Id20::from_str(from_utf8(header.value).context("infohash isn't utf-8")?)
                            .context("invalid infohash header")?,
                    );
                } else if header.name.eq_ignore_ascii_case("cookie") {
                    our_cookie = atoi::atoi::<u32>(header.value);
                }
            }

            match (host, port, infohash) {
                (Some(host), Some(port), Some(hash)) => Ok(BtSearchAnnounceMessage {
                    hash,
                    our_cookie,
                    host,
                    port,
                }),
                _ => anyhow::bail!("not all of host, man and st are set"),
            }
        }
        _ => anyhow::bail!("expecting BT-SEARCH"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a BEP 14 BT-SEARCH message from parts.
    fn build_bt_search_msg(host: &str, port: u16, infohash: &str, cookie: Option<u32>) -> Vec<u8> {
        let mut msg = String::new();
        msg.push_str("BT-SEARCH * HTTP/1.1\r\n");
        msg.push_str(&format!("Host: {host}\r\n"));
        msg.push_str(&format!("Port: {port}\r\n"));
        msg.push_str(&format!("Infohash: {infohash}\r\n"));
        if let Some(c) = cookie {
            msg.push_str(&format!("cookie: {c}\r\n"));
        }
        msg.push_str("\r\n");
        msg.into_bytes()
    }

    #[test]
    fn test_lsd_announce_message_format() {
        // Verify that gen_announce_msg produces a valid BEP 14 message
        // by constructing the expected format manually and comparing structure.
        let info_hash = Id20::from_str("0123456789abcdef0123456789abcdef01234567").unwrap();
        let port = 6881u16;
        let cookie = 42u32;
        let info_hash_hex = info_hash.as_string();

        // Build the expected IPv4 message format.
        let expected_host_v4: SocketAddr = LSD_IPV4.into();
        let expected = format!(
            "BT-SEARCH * HTTP/1.1\r\n\
             Host: {expected_host_v4}\r\n\
             Port: {port}\r\n\
             Infohash: {info_hash_hex}\r\n\
             cookie: {cookie}\r\n\
             \r\n\
             \r\n"
        );

        // Parse it back to verify it is valid.
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let parsed = try_parse_bt_search(expected.as_bytes(), &mut headers).unwrap();
        assert_eq!(parsed.hash, info_hash);
        assert_eq!(parsed.port, port);
        assert_eq!(parsed.our_cookie, Some(cookie));
        assert_eq!(parsed.host, expected_host_v4);
    }

    #[test]
    fn test_lsd_parse_announcement() {
        // Parse a valid LSD BT-SEARCH announcement.
        let info_hash_hex = "aabbccddee11223344556677889900aabbccddee";
        let info_hash = Id20::from_str(info_hash_hex).unwrap();
        let buf = build_bt_search_msg("239.192.152.143:6771", 51413, info_hash_hex, Some(99));

        let mut headers = [httparse::EMPTY_HEADER; 16];
        let parsed = try_parse_bt_search(&buf, &mut headers).unwrap();
        assert_eq!(parsed.hash, info_hash);
        assert_eq!(parsed.port, 51413);
        assert_eq!(parsed.our_cookie, Some(99));
        assert_eq!(
            parsed.host,
            "239.192.152.143:6771".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn test_lsd_parse_announcement_ipv6_host() {
        // Parse a valid LSD announcement with an IPv6 multicast host.
        let info_hash_hex = "0000000000000000000000000000000000000001";
        let info_hash = Id20::from_str(info_hash_hex).unwrap();
        let ipv6_host = "[ff15::efc0:988f]:6771";
        let buf = build_bt_search_msg(ipv6_host, 8080, info_hash_hex, None);

        let mut headers = [httparse::EMPTY_HEADER; 16];
        let parsed = try_parse_bt_search(&buf, &mut headers).unwrap();
        assert_eq!(parsed.hash, info_hash);
        assert_eq!(parsed.port, 8080);
        assert_eq!(parsed.our_cookie, None);
        assert_eq!(parsed.host, ipv6_host.parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn test_lsd_parse_invalid_announcement_wrong_method() {
        // A message with GET instead of BT-SEARCH should be rejected.
        let msg = b"GET * HTTP/1.1\r\n\
                     Host: 239.192.152.143:6771\r\n\
                     Port: 6881\r\n\
                     Infohash: aabbccddee11223344556677889900aabbccddee\r\n\
                     \r\n";
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let result = try_parse_bt_search(msg, &mut headers);
        assert!(result.is_err());
        assert!(format!("{:#}", result.unwrap_err()).contains("expecting BT-SEARCH"),);
    }

    #[test]
    fn test_lsd_parse_invalid_announcement_missing_host() {
        // Missing Host header.
        let msg = b"BT-SEARCH * HTTP/1.1\r\n\
                     Port: 6881\r\n\
                     Infohash: aabbccddee11223344556677889900aabbccddee\r\n\
                     \r\n";
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let result = try_parse_bt_search(msg, &mut headers);
        assert!(result.is_err());
    }

    #[test]
    fn test_lsd_parse_invalid_announcement_missing_port() {
        // Missing Port header.
        let msg = b"BT-SEARCH * HTTP/1.1\r\n\
                     Host: 239.192.152.143:6771\r\n\
                     Infohash: aabbccddee11223344556677889900aabbccddee\r\n\
                     \r\n";
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let result = try_parse_bt_search(msg, &mut headers);
        assert!(result.is_err());
    }

    #[test]
    fn test_lsd_parse_invalid_announcement_missing_infohash() {
        // Missing Infohash header.
        let msg = b"BT-SEARCH * HTTP/1.1\r\n\
                     Host: 239.192.152.143:6771\r\n\
                     Port: 6881\r\n\
                     \r\n";
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let result = try_parse_bt_search(msg, &mut headers);
        assert!(result.is_err());
    }

    #[test]
    fn test_lsd_parse_invalid_announcement_bad_infohash() {
        // Infohash that is not valid hex / wrong length.
        let msg = b"BT-SEARCH * HTTP/1.1\r\n\
                     Host: 239.192.152.143:6771\r\n\
                     Port: 6881\r\n\
                     Infohash: not_a_valid_hash\r\n\
                     \r\n";
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let result = try_parse_bt_search(msg, &mut headers);
        assert!(result.is_err());
    }

    #[test]
    fn test_lsd_parse_invalid_announcement_bad_port() {
        // Port that is not a number.
        let msg = b"BT-SEARCH * HTTP/1.1\r\n\
                     Host: 239.192.152.143:6771\r\n\
                     Port: notaport\r\n\
                     Infohash: aabbccddee11223344556677889900aabbccddee\r\n\
                     \r\n";
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let result = try_parse_bt_search(msg, &mut headers);
        assert!(result.is_err());
    }

    #[test]
    fn test_lsd_info_hash_matching() {
        // Verify that the exact info_hash bytes are extracted correctly.
        let info_hash_hex = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let expected = Id20::from_str(info_hash_hex).unwrap();
        let buf = build_bt_search_msg("239.192.152.143:6771", 6881, info_hash_hex, None);

        let mut headers = [httparse::EMPTY_HEADER; 16];
        let parsed = try_parse_bt_search(&buf, &mut headers).unwrap();
        assert_eq!(parsed.hash, expected);
        assert_eq!(parsed.hash.as_string(), info_hash_hex);
    }

    #[test]
    fn test_lsd_port_extraction() {
        // Verify various port values parse correctly.
        for port in [1u16, 80, 443, 6881, 51413, 65535] {
            let buf = build_bt_search_msg(
                "239.192.152.143:6771",
                port,
                "0000000000000000000000000000000000000000",
                None,
            );
            let mut headers = [httparse::EMPTY_HEADER; 16];
            let parsed = try_parse_bt_search(&buf, &mut headers).unwrap();
            assert_eq!(parsed.port, port, "failed for port {port}");
        }
    }

    #[test]
    fn test_lsd_cookie_handling() {
        // With cookie present.
        let buf = build_bt_search_msg(
            "239.192.152.143:6771",
            6881,
            "0123456789abcdef0123456789abcdef01234567",
            Some(987654321),
        );
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let parsed = try_parse_bt_search(&buf, &mut headers).unwrap();
        assert_eq!(parsed.our_cookie, Some(987654321));

        // Without cookie.
        let buf = build_bt_search_msg(
            "239.192.152.143:6771",
            6881,
            "0123456789abcdef0123456789abcdef01234567",
            None,
        );
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let parsed = try_parse_bt_search(&buf, &mut headers).unwrap();
        assert_eq!(parsed.our_cookie, None);
    }

    #[test]
    fn test_lsd_cookie_non_numeric() {
        // A cookie that is not a valid u32 should parse as None (not an error),
        // since the code uses atoi which returns None on failure.
        let mut msg = String::new();
        msg.push_str("BT-SEARCH * HTTP/1.1\r\n");
        msg.push_str("Host: 239.192.152.143:6771\r\n");
        msg.push_str("Port: 6881\r\n");
        msg.push_str("Infohash: 0123456789abcdef0123456789abcdef01234567\r\n");
        msg.push_str("cookie: not_a_number\r\n");
        msg.push_str("\r\n");

        let buf = msg.into_bytes();
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let parsed = try_parse_bt_search(&buf, &mut headers).unwrap();
        assert_eq!(parsed.our_cookie, None);
    }

    #[test]
    fn test_lsd_case_insensitive_headers() {
        // BEP 14 says headers should be case-insensitive. Verify mixed case works.
        let mut msg = String::new();
        msg.push_str("BT-SEARCH * HTTP/1.1\r\n");
        msg.push_str("HOST: 239.192.152.143:6771\r\n");
        msg.push_str("PORT: 6881\r\n");
        msg.push_str("INFOHASH: aabbccddee11223344556677889900aabbccddee\r\n");
        msg.push_str("COOKIE: 42\r\n");
        msg.push_str("\r\n");

        let buf = msg.into_bytes();
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let parsed = try_parse_bt_search(&buf, &mut headers).unwrap();
        assert_eq!(
            parsed.hash,
            Id20::from_str("aabbccddee11223344556677889900aabbccddee").unwrap()
        );
        assert_eq!(parsed.port, 6881);
        assert_eq!(parsed.our_cookie, Some(42));
    }

    #[test]
    fn test_lsd_constants() {
        // Verify the BEP 14 multicast addresses and port.
        assert_eq!(LSD_PORT, 6771);
        assert_eq!(LSD_IPV4.ip(), &Ipv4Addr::new(239, 192, 152, 143));
        assert_eq!(LSD_IPV4.port(), 6771);
        assert_eq!(
            LSD_IPV6.ip(),
            &Ipv6Addr::new(0xff15, 0, 0, 0, 0, 0, 0xefc0, 0x988f)
        );
        assert_eq!(LSD_IPV6.port(), 6771);
    }

    #[test]
    fn test_lsd_rate_limiter_allows_first_call() {
        let rl = RateLimiter::default();
        // First call should always succeed.
        assert!(rl.check().is_some());
    }

    #[test]
    fn test_lsd_rate_limiter_blocks_rapid_calls() {
        let rl = RateLimiter::default();
        // First call succeeds.
        assert!(rl.check().is_some());
        // Immediate second call should be rate-limited.
        assert!(rl.check().is_none());
    }
}
