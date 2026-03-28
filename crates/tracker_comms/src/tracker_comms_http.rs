use buffers::ByteBuf;
use itertools::Either;
use serde::Deserializer;
use serde_derive::Deserialize;
use serde_with::serde_as;
use std::{
    marker::PhantomData,
    net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6},
};

use librtbit_core::{
    compact_ip::{CompactListInBuffer, CompactSerialize, CompactSerializeFixedLen},
    hash_id::Id20,
};

#[derive(Clone, Copy)]
pub enum TrackerRequestEvent {
    Started,
    Stopped,
    Completed,
}

pub struct TrackerRequest<'a> {
    pub info_hash: &'a Id20,
    pub peer_id: &'a Id20,
    pub event: Option<TrackerRequestEvent>,
    pub port: u16,
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    pub compact: bool,
    pub no_peer_id: bool,

    pub ip: Option<IpAddr>,
    pub numwant: Option<usize>,
    pub key: Option<u32>,
    pub trackerid: Option<&'a str>,
}

#[derive(Deserialize, Debug)]
pub struct TrackerError<'a> {
    #[serde(rename = "failure reason", borrow)]
    pub failure_reason: ByteBuf<'a>,
}

pub enum Peers<'a, AddrType> {
    DictPeers(Vec<SocketAddr>),
    Compact(CompactListInBuffer<ByteBuf<'a>, AddrType>),
}

impl<'a, AddrType> std::fmt::Debug for Peers<'a, AddrType>
where
    AddrType:
        std::fmt::Debug + CompactSerialize + CompactSerializeFixedLen + Copy + Into<SocketAddr>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<'a, AddrType> Default for Peers<'a, AddrType> {
    fn default() -> Self {
        Self::DictPeers(Default::default())
    }
}

impl<'a, AddrType> Peers<'a, AddrType>
where
    AddrType: CompactSerialize + CompactSerializeFixedLen + Copy + Into<SocketAddr>,
{
    fn iter(&self) -> impl Iterator<Item = SocketAddr> {
        match self {
            Peers::DictPeers(a) => Either::Left(a.iter().copied()),
            Peers::Compact(l) => Either::Right(l.iter().map(Into::into)),
        }
    }
}

impl<'a, 'de, AddrType> serde::de::Deserialize<'de> for Peers<'a, AddrType>
where
    AddrType: CompactSerialize + CompactSerializeFixedLen + Into<SocketAddr> + 'static,
    'de: 'a,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[serde_as]
        #[derive(Deserialize)]
        struct DictPeer {
            #[serde_as(as = "serde_with::DisplayFromStr")]
            ip: IpAddr,
            port: u16,
        }

        struct Visitor<'a, 'de, AddrType> {
            phantom: std::marker::PhantomData<&'de &'a AddrType>,
        }
        impl<'a, 'de, AddrType> serde::de::Visitor<'de> for Visitor<'a, 'de, AddrType>
        where
            AddrType: CompactSerialize + CompactSerializeFixedLen + Into<SocketAddr>,
        {
            type Value = Peers<'de, AddrType>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a list of peers in dict or compact format")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut addrs = Vec::new();
                while let Some(peer) = seq.next_element::<DictPeer>()? {
                    addrs.push(SocketAddr::from((peer.ip, peer.port)));
                    if addrs.len() >= 10_000 {
                        break;
                    }
                }
                Ok(Peers::DictPeers(addrs))
            }

            fn visit_borrowed_bytes<E>(self, v: &'de [u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Peers::Compact(CompactListInBuffer::new_from_buf(v.into())))
            }
        }
        deserializer.deserialize_any(Visitor {
            phantom: PhantomData,
        })
    }
}

#[derive(Deserialize, Debug)]
pub struct TrackerResponse<'a> {
    #[allow(dead_code)]
    #[serde(rename = "warning message", borrow)]
    pub warning_message: Option<ByteBuf<'a>>,
    #[allow(dead_code)]
    #[serde(default)]
    pub complete: u64,
    pub interval: u64,
    #[allow(dead_code)]
    #[serde(rename = "min interval")]
    pub min_interval: Option<u64>,
    #[allow(dead_code)]
    pub tracker_id: Option<ByteBuf<'a>>,
    #[allow(dead_code)]
    #[serde(default)]
    pub incomplete: u64,
    #[serde(borrow)]
    pub peers: Peers<'a, SocketAddrV4>,
    #[serde(default, borrow)]
    pub peers6: Peers<'a, SocketAddrV6>,
}

impl TrackerResponse<'_> {
    pub fn iter_peers(&self) -> impl Iterator<Item = SocketAddr> {
        self.peers.iter().chain(self.peers6.iter())
    }
}

impl TrackerRequest<'_> {
    pub fn as_querystring(&self) -> String {
        use std::fmt::Write;
        use urlencoding as u;
        let mut s = String::new();
        s.push_str("info_hash=");
        s.push_str(u::encode_binary(&self.info_hash.0).as_ref());
        s.push_str("&peer_id=");
        s.push_str(u::encode_binary(&self.peer_id.0).as_ref());
        if let Some(event) = self.event {
            write!(
                s,
                "&event={}",
                match event {
                    TrackerRequestEvent::Started => "started",
                    TrackerRequestEvent::Stopped => "stopped",
                    TrackerRequestEvent::Completed => "completed",
                }
            )
            .unwrap();
        }
        write!(s, "&port={}", self.port).unwrap();
        write!(s, "&uploaded={}", self.uploaded).unwrap();
        write!(s, "&downloaded={}", self.downloaded).unwrap();
        write!(s, "&left={}", self.left).unwrap();
        write!(s, "&compact={}", if self.compact { 1 } else { 0 }).unwrap();
        write!(s, "&no_peer_id={}", if self.no_peer_id { 1 } else { 0 }).unwrap();
        if let Some(ip) = &self.ip {
            write!(s, "&ip={ip}").unwrap();
        }
        if let Some(numwant) = &self.numwant {
            write!(s, "&numwant={numwant}").unwrap();
        }
        if let Some(key) = &self.key {
            write!(s, "&key={key}").unwrap();
        }
        if let Some(trackerid) = &self.trackerid {
            write!(s, "&trackerid={trackerid}").unwrap();
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_serialize() {
        let info_hash = Id20::new([
            1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        ]);
        let peer_id = info_hash;
        let request = TrackerRequest {
            info_hash: &info_hash,
            peer_id: &peer_id,
            port: 6881,
            uploaded: 0,
            downloaded: 0,
            left: 1024 * 1024,
            compact: true,
            no_peer_id: false,
            event: Some(TrackerRequestEvent::Started),
            ip: Some("127.0.0.1".parse().unwrap()),
            numwant: None,
            key: None,
            trackerid: None,
        };
        dbg!(request.as_querystring());
    }

    #[test]
    fn test_parse_tracker_response_compact() {
        let data = b"d8:intervali1800e5:peers6:iiiipp6:peers618:iiiiiiiiiiiiiiiippe";
        let response = bencode::from_bytes::<TrackerResponse>(data).unwrap();
        assert_eq!(
            response.iter_peers().collect::<Vec<_>>(),
            vec![
                "105.105.105.105:28784".parse().unwrap(),
                "[6969:6969:6969:6969:6969:6969:6969:6969]:28784"
                    .parse()
                    .unwrap()
            ]
        );
        dbg!(response);
    }

    #[test]
    fn parse_peers_dict() {
        let buf = b"ld2:ip9:127.0.0.14:porti100eed2:ip39:6969:6969:6969:6969:6969:6969:6969:69694:porti101eee";
        dbg!(bencode::dyn_from_bytes::<ByteBuf>(buf).unwrap());
        let peers = bencode::from_bytes::<Peers<SocketAddrV4>>(buf).unwrap();
        assert_eq!(
            peers.iter().collect::<Vec<_>>(),
            vec![
                "127.0.0.1:100".parse().unwrap(),
                "[6969:6969:6969:6969:6969:6969:6969:6969]:101"
                    .parse()
                    .unwrap()
            ]
        );
    }

    #[test]
    fn test_parse_tracker_response_with_peers() {
        // A normal tracker response with multiple compact IPv4 peers.
        // Each peer is 6 bytes: 4 bytes IP + 2 bytes port (big-endian).
        // Peer 1: 192.168.1.1:6881  => [192, 168, 1, 1, 0x1A, 0xE1]
        // Peer 2: 10.0.0.1:51413    => [10, 0, 0, 1, 0xC8, 0xD5]
        let peer1 = [192u8, 168, 1, 1, 0x1A, 0xE1];
        let peer2 = [10u8, 0, 0, 1, 0xC8, 0xD5];
        let mut peers_bytes = Vec::new();
        peers_bytes.extend_from_slice(&peer1);
        peers_bytes.extend_from_slice(&peer2);

        // Build bencode: d8:completei50e10:incompletei10e8:intervali900e5:peers12:<bytes>e
        let mut data = Vec::new();
        data.extend_from_slice(b"d8:completei50e10:incompletei10e8:intervali900e5:peers");
        data.extend_from_slice(format!("{}:", peers_bytes.len()).as_bytes());
        data.extend_from_slice(&peers_bytes);
        data.extend_from_slice(b"e");

        let response = bencode::from_bytes::<TrackerResponse>(&data).unwrap();
        assert_eq!(response.interval, 900);
        assert_eq!(response.complete, 50);
        assert_eq!(response.incomplete, 10);

        let addrs: Vec<SocketAddr> = response.iter_peers().collect();
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0], "192.168.1.1:6881".parse().unwrap());
        assert_eq!(addrs[1], "10.0.0.1:51413".parse().unwrap());
    }

    #[test]
    fn test_parse_tracker_response_failure() {
        // A tracker failure response contains "failure reason" key.
        let data = b"d14:failure reason15:torrent unknowne";
        let error = bencode::from_bytes::<TrackerError>(data).unwrap();
        assert_eq!(
            std::str::from_utf8(error.failure_reason.0).unwrap(),
            "torrent unknown"
        );

        // Parsing as TrackerResponse should fail since required fields are missing.
        let result = bencode::from_bytes::<TrackerResponse>(data);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_tracker_response_warning() {
        // A response that includes a warning_message alongside normal fields.
        // Compact peer: 127.0.0.1:80 => [127, 0, 0, 1, 0, 80]
        let peer_bytes = [127u8, 0, 0, 1, 0, 80];
        let mut data = Vec::new();
        data.extend_from_slice(b"d");
        data.extend_from_slice(b"8:completei5e");
        data.extend_from_slice(b"10:incompletei3e");
        data.extend_from_slice(b"8:intervali1800e");
        data.extend_from_slice(b"5:peers6:");
        data.extend_from_slice(&peer_bytes);
        data.extend_from_slice(b"15:warning message11:please note");
        data.extend_from_slice(b"e");

        let response = bencode::from_bytes::<TrackerResponse>(&data).unwrap();
        assert!(response.warning_message.is_some());
        assert_eq!(
            std::str::from_utf8(response.warning_message.unwrap().0).unwrap(),
            "please note"
        );
        assert_eq!(response.interval, 1800);
        let addrs: Vec<SocketAddr> = response.iter_peers().collect();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0], "127.0.0.1:80".parse().unwrap());
    }

    #[test]
    fn test_parse_tracker_response_no_peers() {
        // A response with an empty compact peer list.
        let data = b"d8:completei0e10:incompletei0e8:intervali3600e5:peers0:e";
        let response = bencode::from_bytes::<TrackerResponse>(data).unwrap();
        assert_eq!(response.interval, 3600);
        assert_eq!(response.complete, 0);
        assert_eq!(response.incomplete, 0);
        let addrs: Vec<SocketAddr> = response.iter_peers().collect();
        assert!(addrs.is_empty());
    }

    #[test]
    fn test_parse_compact_peer_list_ipv4() {
        // Compact IPv4 peer list: 3 peers, each 6 bytes.
        // 1.2.3.4:256   => [1, 2, 3, 4, 1, 0]
        // 5.6.7.8:1024  => [5, 6, 7, 8, 4, 0]
        // 255.255.255.255:65535 => [255, 255, 255, 255, 255, 255]
        let mut buf = Vec::new();
        buf.extend_from_slice(&[1, 2, 3, 4, 1, 0]);
        buf.extend_from_slice(&[5, 6, 7, 8, 4, 0]);
        buf.extend_from_slice(&[255, 255, 255, 255, 255, 255]);

        // Wrap in bencode string format for deserialization.
        let mut bencoded = Vec::new();
        bencoded.extend_from_slice(format!("{}:", buf.len()).as_bytes());
        bencoded.extend_from_slice(&buf);

        let peers = bencode::from_bytes::<Peers<SocketAddrV4>>(&bencoded).unwrap();
        let addrs: Vec<SocketAddr> = peers.iter().collect();
        assert_eq!(addrs.len(), 3);
        assert_eq!(addrs[0], "1.2.3.4:256".parse().unwrap());
        assert_eq!(addrs[1], "5.6.7.8:1024".parse().unwrap());
        assert_eq!(addrs[2], "255.255.255.255:65535".parse().unwrap());
    }

    #[test]
    fn test_parse_compact_peer_list_ipv6() {
        // Compact IPv6 peer list: each peer is 18 bytes (16 bytes IP + 2 bytes port).
        // ::1 port 6881 => [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1, 0x1A, 0xE1]
        let mut buf = Vec::new();
        // IPv6 ::1
        buf.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        // port 6881
        buf.extend_from_slice(&[0x1A, 0xE1]);

        let mut bencoded = Vec::new();
        bencoded.extend_from_slice(format!("{}:", buf.len()).as_bytes());
        bencoded.extend_from_slice(&buf);

        let peers = bencode::from_bytes::<Peers<SocketAddrV6>>(&bencoded).unwrap();
        let addrs: Vec<SocketAddr> = peers.iter().collect();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0], "[::1]:6881".parse().unwrap());
    }

    #[test]
    fn test_serialize_announce_request() {
        let info_hash = Id20::new([0xAAu8; 20]);
        let peer_id = Id20::new([0xBBu8; 20]);
        let request = TrackerRequest {
            info_hash: &info_hash,
            peer_id: &peer_id,
            port: 6881,
            uploaded: 1000,
            downloaded: 5000,
            left: 50000,
            compact: true,
            no_peer_id: true,
            event: Some(TrackerRequestEvent::Completed),
            ip: None,
            numwant: Some(50),
            key: Some(12345),
            trackerid: Some("TRACKER001"),
        };

        let qs = request.as_querystring();

        // Verify all required fields are present.
        assert!(qs.contains("info_hash="));
        assert!(qs.contains("peer_id="));
        assert!(qs.contains("&port=6881"));
        assert!(qs.contains("&uploaded=1000"));
        assert!(qs.contains("&downloaded=5000"));
        assert!(qs.contains("&left=50000"));
        assert!(qs.contains("&compact=1"));
        assert!(qs.contains("&no_peer_id=1"));
        assert!(qs.contains("&event=completed"));
        assert!(qs.contains("&numwant=50"));
        assert!(qs.contains("&key=12345"));
        assert!(qs.contains("&trackerid=TRACKER001"));
        // ip should not be present when None.
        assert!(!qs.contains("&ip="));
    }

    #[test]
    fn test_serialize_announce_request_minimal() {
        // Test with no optional fields set.
        let info_hash = Id20::new([0x01u8; 20]);
        let peer_id = Id20::new([0x02u8; 20]);
        let request = TrackerRequest {
            info_hash: &info_hash,
            peer_id: &peer_id,
            port: 12345,
            uploaded: 0,
            downloaded: 0,
            left: 0,
            compact: false,
            no_peer_id: false,
            event: None,
            ip: None,
            numwant: None,
            key: None,
            trackerid: None,
        };

        let qs = request.as_querystring();
        assert!(qs.contains("&port=12345"));
        assert!(qs.contains("&compact=0"));
        assert!(qs.contains("&no_peer_id=0"));
        // Optional fields should be absent.
        assert!(!qs.contains("&event="));
        assert!(!qs.contains("&ip="));
        assert!(!qs.contains("&numwant="));
        assert!(!qs.contains("&key="));
        assert!(!qs.contains("&trackerid="));
    }

    #[test]
    fn test_serialize_announce_request_events() {
        let info_hash = Id20::new([0x00u8; 20]);
        let peer_id = Id20::new([0x00u8; 20]);

        let make_request = |event| TrackerRequest {
            info_hash: &info_hash,
            peer_id: &peer_id,
            port: 1,
            uploaded: 0,
            downloaded: 0,
            left: 0,
            compact: true,
            no_peer_id: false,
            event,
            ip: None,
            numwant: None,
            key: None,
            trackerid: None,
        };

        let qs = make_request(Some(TrackerRequestEvent::Started)).as_querystring();
        assert!(qs.contains("&event=started"));

        let qs = make_request(Some(TrackerRequestEvent::Stopped)).as_querystring();
        assert!(qs.contains("&event=stopped"));

        let qs = make_request(Some(TrackerRequestEvent::Completed)).as_querystring();
        assert!(qs.contains("&event=completed"));

        let qs = make_request(None).as_querystring();
        assert!(!qs.contains("&event="));
    }

    #[test]
    fn test_parse_tracker_response_with_dict_peers() {
        // Non-compact (dictionary) peer format.
        let data = b"d8:intervali600e5:peersld2:ip9:127.0.0.14:porti6881eed2:ip11:192.168.1.14:porti51413eeee";
        let response = bencode::from_bytes::<TrackerResponse>(data).unwrap();
        assert_eq!(response.interval, 600);
        let addrs: Vec<SocketAddr> = response.iter_peers().collect();
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0], "127.0.0.1:6881".parse().unwrap());
        assert_eq!(addrs[1], "192.168.1.1:51413".parse().unwrap());
    }

    #[test]
    fn test_parse_tracker_response_with_min_interval() {
        // Response with both interval and min_interval.
        let data = b"d8:completei10e10:incompletei5e8:intervali1800e12:min intervali900e5:peers0:e";
        let response = bencode::from_bytes::<TrackerResponse>(data).unwrap();
        assert_eq!(response.interval, 1800);
        assert_eq!(response.min_interval, Some(900));
    }

    #[test]
    fn test_parse_tracker_response_with_tracker_id() {
        // Response that includes a tracker_id field.
        let data = b"d8:intervali1800e5:peers0:10:tracker_id7:abc1234e";
        let response = bencode::from_bytes::<TrackerResponse>(data).unwrap();
        assert!(response.tracker_id.is_some());
        assert_eq!(
            std::str::from_utf8(response.tracker_id.unwrap().0).unwrap(),
            "abc1234"
        );
    }

    #[test]
    fn test_parse_tracker_response_ipv4_and_ipv6_combined() {
        // Response with both peers (IPv4) and peers6 (IPv6) compact lists.
        // IPv4 peer: 10.0.0.1:80 => [10, 0, 0, 1, 0, 80]
        let ipv4_peer = [10u8, 0, 0, 1, 0, 80];
        // IPv6 peer: ::1 port 443 => [0..0,1, 0x01, 0xBB]
        let mut ipv6_peer = vec![0u8; 15];
        ipv6_peer.push(1); // ::1
        ipv6_peer.extend_from_slice(&[0x01, 0xBB]); // port 443

        let mut data = Vec::new();
        data.extend_from_slice(b"d8:intervali60e5:peers6:");
        data.extend_from_slice(&ipv4_peer);
        data.extend_from_slice(b"6:peers618:");
        data.extend_from_slice(&ipv6_peer);
        data.extend_from_slice(b"e");

        let response = bencode::from_bytes::<TrackerResponse>(&data).unwrap();
        let addrs: Vec<SocketAddr> = response.iter_peers().collect();
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0], "10.0.0.1:80".parse().unwrap());
        assert_eq!(addrs[1], "[::1]:443".parse().unwrap());
    }
}
