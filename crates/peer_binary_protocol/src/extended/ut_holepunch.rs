use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// BEP 55: Holepunch Extension
///
/// Binary message format:
///   msg_type:   1 byte  (0=Rendezvous, 1=Connect, 2=Error)
///   addr_type:  1 byte  (0=IPv4, 1=IPv6)
///   addr:       4 bytes (IPv4) or 16 bytes (IPv6)
///   port:       2 bytes (big-endian)
///   err_code:   4 bytes (big-endian, only present for Error messages)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HolepunchMsgType {
    Rendezvous = 0,
    Connect = 1,
    Error = 2,
}

impl HolepunchMsgType {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Rendezvous),
            1 => Some(Self::Connect),
            2 => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HolepunchErrorCode {
    NoSuchPeer = 1,
    NotConnected = 2,
    NoSupport = 3,
    NoSelf = 4,
}

impl HolepunchErrorCode {
    fn from_u32(v: u32) -> Option<Self> {
        match v {
            1 => Some(Self::NoSuchPeer),
            2 => Some(Self::NotConnected),
            3 => Some(Self::NoSupport),
            4 => Some(Self::NoSelf),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolepunchMessage {
    pub msg_type: HolepunchMsgType,
    pub addr: SocketAddr,
    pub error_code: Option<HolepunchErrorCode>,
}

impl HolepunchMessage {
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(self.msg_type as u8);
        match self.addr.ip() {
            IpAddr::V4(ip) => {
                buf.push(0); // addr_type = IPv4
                buf.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                buf.push(1); // addr_type = IPv6
                buf.extend_from_slice(&ip.octets());
            }
        }
        buf.extend_from_slice(&self.addr.port().to_be_bytes());
        if let Some(err) = self.error_code {
            buf.extend_from_slice(&(err as u32).to_be_bytes());
        }
        buf
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        if data.len() < 2 {
            return Err("holepunch message too short".into());
        }

        let msg_type = HolepunchMsgType::from_u8(data[0]).ok_or("unknown holepunch msg_type")?;
        let addr_type = data[1];

        let (ip, rest) = match addr_type {
            0 => {
                if data.len() < 2 + 4 + 2 {
                    return Err("holepunch IPv4 message too short".into());
                }
                let octets: [u8; 4] = data[2..6].try_into().unwrap();
                (IpAddr::V4(Ipv4Addr::from(octets)), &data[6..])
            }
            1 => {
                if data.len() < 2 + 16 + 2 {
                    return Err("holepunch IPv6 message too short".into());
                }
                let octets: [u8; 16] = data[2..18].try_into().unwrap();
                (IpAddr::V6(Ipv6Addr::from(octets)), &data[18..])
            }
            _ => return Err(format!("unknown holepunch addr_type {addr_type}")),
        };

        if rest.len() < 2 {
            return Err("holepunch message missing port".into());
        }
        let port = u16::from_be_bytes([rest[0], rest[1]]);
        let rest = &rest[2..];

        let error_code = if msg_type == HolepunchMsgType::Error {
            if rest.len() < 4 {
                return Err("holepunch error message missing error_code".into());
            }
            let code = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
            Some(
                HolepunchErrorCode::from_u32(code)
                    .ok_or_else(|| format!("unknown holepunch error_code {code}"))?,
            )
        } else {
            None
        };

        Ok(HolepunchMessage {
            msg_type,
            addr: SocketAddr::new(ip, port),
            error_code,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_rendezvous_ipv4() {
        let msg = HolepunchMessage {
            msg_type: HolepunchMsgType::Rendezvous,
            addr: "127.0.0.1:6881".parse().unwrap(),
            error_code: None,
        };
        let bytes = msg.serialize();
        let decoded = HolepunchMessage::deserialize(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_connect_ipv6() {
        let msg = HolepunchMessage {
            msg_type: HolepunchMsgType::Connect,
            addr: "[::1]:6881".parse().unwrap(),
            error_code: None,
        };
        let bytes = msg.serialize();
        let decoded = HolepunchMessage::deserialize(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_error_ipv4() {
        let msg = HolepunchMessage {
            msg_type: HolepunchMsgType::Error,
            addr: "10.0.0.1:12345".parse().unwrap(),
            error_code: Some(HolepunchErrorCode::NotConnected),
        };
        let bytes = msg.serialize();
        let decoded = HolepunchMessage::deserialize(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_error_ipv6() {
        let msg = HolepunchMessage {
            msg_type: HolepunchMsgType::Error,
            addr: "[fe80::1]:443".parse().unwrap(),
            error_code: Some(HolepunchErrorCode::NoSuchPeer),
        };
        let bytes = msg.serialize();
        let decoded = HolepunchMessage::deserialize(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_all_error_codes_roundtrip() {
        for code in [
            HolepunchErrorCode::NoSuchPeer,
            HolepunchErrorCode::NotConnected,
            HolepunchErrorCode::NoSupport,
            HolepunchErrorCode::NoSelf,
        ] {
            let msg = HolepunchMessage {
                msg_type: HolepunchMsgType::Error,
                addr: "1.2.3.4:5678".parse().unwrap(),
                error_code: Some(code),
            };
            let bytes = msg.serialize();
            let decoded = HolepunchMessage::deserialize(&bytes).unwrap();
            assert_eq!(msg, decoded);
        }
    }

    #[test]
    fn test_deserialize_too_short() {
        assert!(HolepunchMessage::deserialize(&[]).is_err());
        assert!(HolepunchMessage::deserialize(&[0]).is_err());
    }

    #[test]
    fn test_deserialize_unknown_msg_type() {
        assert!(HolepunchMessage::deserialize(&[99, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn test_deserialize_unknown_addr_type() {
        assert!(HolepunchMessage::deserialize(&[0, 99, 0, 0, 0, 0, 0, 0]).is_err());
    }
}
