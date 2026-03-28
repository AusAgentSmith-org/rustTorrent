use std::{
    io::Write,
    marker::PhantomData,
    net::{SocketAddr, SocketAddrV4, SocketAddrV6},
};

use bencode::{ByteBuf, ByteBufOwned};
use buffers::ByteBufT;
use bytes::Bytes;
use clone_to_owned::CloneToOwned;
use librtbit_core::{
    compact_ip::{
        Compact, CompactListInBuffer, CompactSerialize, CompactSerializeFixedLen, CompactSocketAddr,
    },
    hash_id::Id20,
};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{IgnoredAny, Unexpected},
};

#[derive(Debug)]
enum MessageType {
    Request,
    Response,
    Error,
}

impl<'de> Deserialize<'de> for MessageType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = MessageType;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(formatter, r#""q", "e" or "r" bencode string"#)
            }
            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let msg = match v {
                    b"q" => MessageType::Request,
                    b"r" => MessageType::Response,
                    b"e" => MessageType::Error,
                    _ => return Err(E::invalid_value(Unexpected::Bytes(v), &self)),
                };
                Ok(msg)
            }
        }
        deserializer.deserialize_bytes(Visitor {})
    }
}

impl Serialize for MessageType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            MessageType::Request => serializer.serialize_bytes(b"q"),
            MessageType::Response => serializer.serialize_bytes(b"r"),
            MessageType::Error => serializer.serialize_bytes(b"e"),
        }
    }
}

#[derive(Debug)]
pub struct ErrorDescription<BufT> {
    pub code: i32,
    pub description: BufT,
}

impl<BufT> CloneToOwned for ErrorDescription<BufT>
where
    BufT: CloneToOwned,
{
    type Target = ErrorDescription<<BufT as CloneToOwned>::Target>;

    fn clone_to_owned(&self, within_buffer: Option<&Bytes>) -> Self::Target {
        ErrorDescription {
            code: self.code,
            description: self.description.clone_to_owned(within_buffer),
        }
    }
}

impl<BufT> Serialize for ErrorDescription<BufT>
where
    BufT: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(2))?;
        seq.serialize_element(&self.code)?;
        seq.serialize_element(&self.description)?;
        seq.end()
    }
}

impl<'de, BufT> Deserialize<'de> for ErrorDescription<BufT>
where
    BufT: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor<BufT> {
            phantom: PhantomData<BufT>,
        }
        impl<'de, BufT> serde::de::Visitor<'de> for Visitor<BufT>
        where
            BufT: Deserialize<'de>,
        {
            type Value = ErrorDescription<BufT>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(formatter, r#"a list [i32, string]"#)
            }
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                use serde::de::Error;
                let code = match seq.next_element::<i32>()? {
                    Some(code) => code,
                    None => return Err(A::Error::invalid_length(0, &self)),
                };
                let description = match seq.next_element::<BufT>()? {
                    Some(code) => code,
                    None => return Err(A::Error::invalid_length(1, &self)),
                };
                // The type doesn't matter here, we are just making sure the list is over.
                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(A::Error::invalid_length(3, &self));
                }
                Ok(ErrorDescription { code, description })
            }
        }
        deserializer.deserialize_seq(Visitor {
            phantom: PhantomData,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct RawMessage<BufT, Args = IgnoredAny, Resp = IgnoredAny> {
    #[serde(rename = "y")]
    message_type: MessageType,
    #[serde(rename = "t")]
    transaction_id: BufT,
    #[serde(rename = "e", skip_serializing_if = "Option::is_none")]
    error: Option<ErrorDescription<BufT>>,
    #[serde(rename = "r", skip_serializing_if = "Option::is_none")]
    response: Option<Resp>,
    #[serde(rename = "q", skip_serializing_if = "Option::is_none")]
    method_name: Option<BufT>,
    #[serde(rename = "a", skip_serializing_if = "Option::is_none")]
    arguments: Option<Args>,
    #[serde(rename = "v", skip_serializing_if = "Option::is_none")]
    version: Option<BufT>,
    #[serde(rename = "ip", skip_serializing_if = "Option::is_none")]
    ip: Option<CompactSocketAddr>,
}

pub struct Node<A> {
    pub id: Id20,
    pub addr: A,
}

impl<A: Into<SocketAddr> + Copy> Node<A> {
    pub fn as_socketaddr(&self) -> Node<SocketAddr> {
        Node {
            id: self.id,
            addr: self.addr.into(),
        }
    }
}

pub type CompactNodeInfo<Buf, A> = CompactListInBuffer<Buf, Node<A>>;
pub type CompactNodeInfoOwned<A> = CompactNodeInfo<ByteBufOwned, A>;

impl CompactSerialize for Node<SocketAddrV4> {
    type Slice = [u8; 26];

    fn expecting() -> &'static str {
        "26 bytes"
    }

    fn as_slice(&self) -> Self::Slice {
        let mut data = [0u8; 26];
        data[..20].copy_from_slice(&self.id.0);
        data[20..26].copy_from_slice(self.addr.as_slice().as_ref());
        data
    }

    fn from_slice(buf: &[u8]) -> Option<Self> {
        if buf.len() != 26 {
            return None;
        }
        Some(Self::from_slice_unchecked_len(buf))
    }

    fn from_slice_unchecked_len(buf: &[u8]) -> Self {
        Node {
            id: Id20::from_bytes(&buf[..20]).unwrap(),
            addr: SocketAddrV4::from_slice_unchecked_len(&buf[20..26]),
        }
    }
}

impl<A: CompactSerializeFixedLen> CompactSerializeFixedLen for Node<A> {
    fn fixed_len() -> usize {
        20 + A::fixed_len()
    }
}

impl CompactSerialize for Node<SocketAddrV6> {
    type Slice = [u8; 38];

    fn expecting() -> &'static str {
        "38 bytes"
    }

    fn as_slice(&self) -> Self::Slice {
        let mut data = [0u8; 38];
        data[..20].copy_from_slice(&self.id.0);
        data[20..38].copy_from_slice(self.addr.as_slice().as_ref());
        data
    }

    fn from_slice(buf: &[u8]) -> Option<Self> {
        if buf.len() != 38 {
            return None;
        }
        Some(Self::from_slice_unchecked_len(buf))
    }

    fn from_slice_unchecked_len(buf: &[u8]) -> Self {
        Node {
            id: Id20::from_bytes(&buf[..20]).unwrap(),
            addr: SocketAddrV6::from_slice_unchecked_len(&buf[20..38]),
        }
    }
}

impl<A: core::fmt::Debug> core::fmt::Debug for Node<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}={:?}", self.addr, self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    V4,
    V6,
    Both,
    None,
}

impl Serialize for Want {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Want::V4 => ["n4"][..].serialize(serializer),
            Want::V6 => ["n6"][..].serialize(serializer),
            Want::Both => ["n4", "n6"][..].serialize(serializer),
            Want::None => {
                const EMPTY: [&str; 0] = [];
                EMPTY[..].serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for Want {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Want;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, r#"a list with "n4", "n6" or both"#)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut want_v4 = false;
                let mut want_v6 = false;
                while let Some(item) = seq.next_element::<&[u8]>()? {
                    match item {
                        b"n4" => want_v4 = true,
                        b"n6" => want_v6 = true,
                        _ => continue,
                    }
                }
                match (want_v4, want_v6) {
                    (true, true) => Ok(Want::Both),
                    (true, false) => Ok(Want::V4),
                    (false, true) => Ok(Want::V6),
                    (false, false) => Ok(Want::None),
                }
            }
        }
        deserializer.deserialize_seq(V)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FindNodeRequest {
    pub id: Id20,
    pub target: Id20,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub want: Option<Want>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Response<BufT: ByteBufT> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<CompactSocketAddr>>,
    pub id: Id20,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<CompactNodeInfo<BufT, SocketAddrV4>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes6: Option<CompactNodeInfo<BufT, SocketAddrV6>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<BufT>,
    // BEP 44 mutable item response fields.
    /// The stored bencoded value (BEP 44).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v: Option<BufT>,
    /// 32-byte Ed25519 public key (BEP 44 mutable items).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub k: Option<BufT>,
    /// 64-byte Ed25519 signature (BEP 44 mutable items).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sig: Option<BufT>,
    /// Sequence number (BEP 44 mutable items).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetPeersRequest {
    pub id: Id20,
    pub info_hash: Id20,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub want: Option<Want>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PingRequest {
    pub id: Id20,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnnouncePeer<BufT> {
    pub id: Id20,
    pub implied_port: u8,
    pub info_hash: Id20,
    pub port: u16,
    pub token: BufT,
}

/// BEP 44: get query arguments for mutable/immutable DHT items.
#[derive(Debug, Serialize, Deserialize)]
pub struct Bep44GetRequest {
    pub id: Id20,
    /// SHA-1 hash of the public key (+ optional salt) for mutable items,
    /// or SHA-1 of the bencoded value for immutable items.
    pub target: Id20,
    /// If present, only return the stored value when its seq > this value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
}

/// BEP 44: put query arguments for mutable DHT items.
#[derive(Debug, Serialize, Deserialize)]
pub struct Bep44PutRequest<BufT> {
    pub id: Id20,
    /// Write token obtained from a prior get response.
    pub token: BufT,
    /// 32-byte Ed25519 public key.
    pub k: BufT,
    /// 64-byte Ed25519 signature.
    pub sig: BufT,
    /// Sequence number.
    pub seq: i64,
    /// The bencoded value to store (max 1000 bytes).
    pub v: BufT,
    /// Optional salt (used in target computation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub salt: Option<BufT>,
    /// Optional compare-and-swap: only overwrite if stored seq == cas.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cas: Option<i64>,
}

#[derive(Debug)]
pub struct Message<BufT: ByteBufT> {
    pub kind: MessageKind<BufT>,
    pub transaction_id: BufT,
    pub version: Option<BufT>,
    pub ip: Option<SocketAddr>,
}

impl Message<ByteBufOwned> {
    // This implies that the transaction id was generated by us.
    pub fn get_our_transaction_id(&self) -> Option<u16> {
        let tid = self.transaction_id.as_ref();
        if tid.len() != 2 {
            return None;
        }
        let tid = ((tid[0] as u16) << 8) + (tid[1] as u16);
        Some(tid)
    }
}

pub enum MessageKind<BufT: ByteBufT> {
    Error(ErrorDescription<BufT>),
    GetPeersRequest(GetPeersRequest),
    FindNodeRequest(FindNodeRequest),
    Response(Response<BufT>),
    PingRequest(PingRequest),
    AnnouncePeer(AnnouncePeer<BufT>),
    Bep44GetRequest(Bep44GetRequest),
    Bep44PutRequest(Bep44PutRequest<BufT>),
}

impl<BufT: ByteBufT> core::fmt::Debug for MessageKind<BufT> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error(e) => write!(f, "{e:?}"),
            Self::GetPeersRequest(r) => write!(f, "{r:?}"),
            Self::FindNodeRequest(r) => write!(f, "{r:?}"),
            Self::Response(r) => write!(f, "{r:?}"),
            Self::PingRequest(r) => write!(f, "{r:?}"),
            Self::AnnouncePeer(r) => write!(f, "{r:?}"),
            Self::Bep44GetRequest(r) => write!(f, "{r:?}"),
            Self::Bep44PutRequest(r) => write!(f, "{r:?}"),
        }
    }
}

pub fn serialize_message<'a, W: Write, BufT: ByteBufT + From<&'a [u8]>>(
    writer: &mut W,
    transaction_id: BufT,
    version: Option<BufT>,
    ip: Option<SocketAddr>,
    kind: MessageKind<BufT>,
) -> crate::Result<()> {
    let ip = ip.map(Compact);
    match kind {
        MessageKind::Error(e) => {
            let msg: RawMessage<BufT, (), ()> = RawMessage {
                message_type: MessageType::Error,
                transaction_id,
                error: Some(e),
                response: None,
                method_name: None,
                version,
                ip,
                arguments: None,
            };
            Ok(bencode::bencode_serialize_to_writer(msg, writer)?)
        }
        MessageKind::GetPeersRequest(req) => {
            let msg: RawMessage<BufT, _, ()> = RawMessage {
                message_type: MessageType::Request,
                transaction_id,
                error: None,
                response: None,
                method_name: Some(BufT::from(b"get_peers")),
                arguments: Some(req),
                ip,
                version,
            };
            Ok(bencode::bencode_serialize_to_writer(msg, writer)?)
        }
        MessageKind::FindNodeRequest(req) => {
            let msg: RawMessage<BufT, _, ()> = RawMessage {
                message_type: MessageType::Request,
                transaction_id,
                error: None,
                response: None,
                method_name: Some(BufT::from(b"find_node")),
                arguments: Some(req),
                ip,
                version,
            };
            Ok(bencode::bencode_serialize_to_writer(msg, writer)?)
        }
        MessageKind::Response(resp) => {
            let msg: RawMessage<BufT, (), _> = RawMessage {
                message_type: MessageType::Response,
                transaction_id,
                error: None,
                response: Some(resp),
                method_name: None,
                arguments: None,
                ip,
                version,
            };
            Ok(bencode::bencode_serialize_to_writer(msg, writer)?)
        }
        MessageKind::PingRequest(ping) => {
            let msg: RawMessage<BufT, _, ()> = RawMessage {
                message_type: MessageType::Request,
                transaction_id,
                error: None,
                response: None,
                method_name: Some(BufT::from(b"ping")),
                arguments: Some(ping),
                ip,
                version,
            };
            Ok(bencode::bencode_serialize_to_writer(msg, writer)?)
        }
        MessageKind::AnnouncePeer(announce) => {
            let msg: RawMessage<BufT, _, ()> = RawMessage {
                message_type: MessageType::Request,
                transaction_id,
                error: None,
                response: None,
                method_name: Some(BufT::from(b"announce_peer")),
                arguments: Some(announce),
                ip,
                version,
            };
            Ok(bencode::bencode_serialize_to_writer(msg, writer)?)
        }
        MessageKind::Bep44GetRequest(req) => {
            let msg: RawMessage<BufT, _, ()> = RawMessage {
                message_type: MessageType::Request,
                transaction_id,
                error: None,
                response: None,
                method_name: Some(BufT::from(b"get")),
                arguments: Some(req),
                ip,
                version,
            };
            Ok(bencode::bencode_serialize_to_writer(msg, writer)?)
        }
        MessageKind::Bep44PutRequest(req) => {
            let msg: RawMessage<BufT, _, ()> = RawMessage {
                message_type: MessageType::Request,
                transaction_id,
                error: None,
                response: None,
                method_name: Some(BufT::from(b"put")),
                arguments: Some(req),
                ip,
                version,
            };
            Ok(bencode::bencode_serialize_to_writer(msg, writer)?)
        }
    }
}

pub fn deserialize_message<'de, BufT>(buf: &'de [u8]) -> anyhow::Result<Message<BufT>>
where
    BufT: ByteBufT + Deserialize<'de>,
{
    let de: RawMessage<ByteBuf> = bencode::from_bytes(buf).map_err(|e| e.into_anyhow())?;
    match de.message_type {
        MessageType::Request => match (&de.arguments, &de.method_name, &de.response, &de.error) {
            (Some(_), Some(method_name), None, None) => match method_name.as_ref() {
                b"find_node" => {
                    let de: RawMessage<BufT, FindNodeRequest> =
                        bencode::from_bytes(buf).map_err(|e| e.into_anyhow())?;
                    Ok(Message {
                        transaction_id: de.transaction_id,
                        version: de.version,
                        ip: de.ip.map(|c| c.0),
                        kind: MessageKind::FindNodeRequest(de.arguments.unwrap()),
                    })
                }
                b"get_peers" => {
                    let de: RawMessage<BufT, GetPeersRequest> =
                        bencode::from_bytes(buf).map_err(|e| e.into_anyhow())?;
                    Ok(Message {
                        transaction_id: de.transaction_id,
                        version: de.version,
                        ip: de.ip.map(|c| c.0),
                        kind: MessageKind::GetPeersRequest(de.arguments.unwrap()),
                    })
                }
                b"ping" => {
                    let de: RawMessage<BufT, PingRequest> =
                        bencode::from_bytes(buf).map_err(|e| e.into_anyhow())?;
                    Ok(Message {
                        transaction_id: de.transaction_id,
                        version: de.version,
                        ip: de.ip.map(|c| c.0),
                        kind: MessageKind::PingRequest(de.arguments.unwrap()),
                    })
                }
                b"announce_peer" => {
                    let de: RawMessage<BufT, AnnouncePeer<BufT>> =
                        bencode::from_bytes(buf).map_err(|e| e.into_anyhow())?;
                    Ok(Message {
                        transaction_id: de.transaction_id,
                        version: de.version,
                        ip: de.ip.map(|c| c.0),
                        kind: MessageKind::AnnouncePeer(de.arguments.unwrap()),
                    })
                }
                b"get" => {
                    let de: RawMessage<BufT, Bep44GetRequest> =
                        bencode::from_bytes(buf).map_err(|e| e.into_anyhow())?;
                    Ok(Message {
                        transaction_id: de.transaction_id,
                        version: de.version,
                        ip: de.ip.map(|c| c.0),
                        kind: MessageKind::Bep44GetRequest(de.arguments.unwrap()),
                    })
                }
                b"put" => {
                    let de: RawMessage<BufT, Bep44PutRequest<BufT>> =
                        bencode::from_bytes(buf).map_err(|e| e.into_anyhow())?;
                    Ok(Message {
                        transaction_id: de.transaction_id,
                        version: de.version,
                        ip: de.ip.map(|c| c.0),
                        kind: MessageKind::Bep44PutRequest(de.arguments.unwrap()),
                    })
                }
                other => anyhow::bail!("unsupported method {:?}", ByteBuf(other)),
            },
            _ => anyhow::bail!(
                "cannot deserialize message as request, expected exactly \"a\" and \"q\" to be set. Message: {:?}",
                de
            ),
        },
        MessageType::Response => match (&de.arguments, &de.method_name, &de.response, &de.error) {
            // some peers are sending method name against the protocol, so ignore it.
            (None, _, Some(_), None) => {
                let de: RawMessage<BufT, IgnoredAny, Response<BufT>> =
                    bencode::from_bytes(buf).map_err(|e| e.into_anyhow())?;
                Ok(Message {
                    transaction_id: de.transaction_id,
                    version: de.version,
                    ip: de.ip.map(|c| c.0),
                    kind: MessageKind::Response(de.response.unwrap()),
                })
            }
            _ => anyhow::bail!(
                "cannot deserialize message as response, expected exactly \"r\" to be set. Message: {:?}",
                de
            ),
        },
        MessageType::Error => match (&de.arguments, &de.method_name, &de.response, &de.error) {
            // some peers are sending method name against the protocol, so ignore it.
            (None, _, None, Some(_)) => {
                let de: RawMessage<BufT, IgnoredAny, Response<BufT>> =
                    bencode::from_bytes(buf).map_err(|e| e.into_anyhow())?;
                Ok(Message {
                    transaction_id: de.transaction_id,
                    version: de.version,
                    ip: de.ip.map(|c| c.0),
                    kind: MessageKind::Error(de.error.unwrap()),
                })
            }
            _ => anyhow::bail!(
                "cannot deserialize message as error, expected exactly \"e\" to be set. Message: {:?}",
                de
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use crate::bprotocol::{self, Want};
    use bencode::{ByteBuf, bencode_serialize_to_writer};

    // Dumped with wireshark.
    const FIND_NODE_REQUEST: &[u8] =
        include_bytes!("../resources/test_requests/find_node_request.bin");
    const GET_PEERS_REQUEST_0: &[u8] =
        include_bytes!("../resources/test_requests/get_peers_request_0.bin");
    const GET_PEERS_REQUEST_1: &[u8] =
        include_bytes!("../resources/test_requests/get_peers_request_1.bin");
    const FIND_NODE_RESPONSE_1: &[u8] =
        include_bytes!("../resources/test_requests/find_node_response_1.bin");
    const FIND_NODE_RESPONSE_2: &[u8] =
        include_bytes!("../resources/test_requests/find_node_response_2.bin");
    const FIND_NODE_RESPONSE_3: &[u8] =
        include_bytes!("../resources/test_requests/find_node_response_3.bin");

    fn write(filename: &str, data: &[u8]) {
        let full = format!("/tmp/{filename}.bin");
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(full)
            .unwrap();
        f.write_all(data).unwrap()
    }

    fn debug_bencode(name: &str, data: &[u8]) {
        println!(
            "{name}: {:#?}",
            bencode::dyn_from_bytes::<ByteBuf>(data).unwrap()
        );
    }

    fn test_deserialize_then_serialize(data: &[u8], name: &'static str) {
        dbg!(bencode::dyn_from_bytes::<ByteBuf>(data).unwrap());
        let bprotocol::Message {
            kind,
            transaction_id,
            version,
            ip,
        } = dbg!(bprotocol::deserialize_message::<ByteBuf>(data).unwrap());
        let mut buf = Vec::new();
        bprotocol::serialize_message(&mut buf, transaction_id, version, ip, kind).unwrap();

        if buf.as_slice() != data {
            write(&format!("{name}-serialized"), buf.as_slice());
            write(&format!("{name}-expected"), data);
            panic!(
                "{} results don't match, dumped to /tmp/{}-*.bin",
                name, name
            )
        }
    }

    #[test]
    fn serialize_then_deserialize_then_serialize_error() {
        let mut buf = Vec::new();
        let transaction_id = ByteBuf(b"123");
        bprotocol::serialize_message(
            &mut buf,
            transaction_id,
            None,
            None,
            bprotocol::MessageKind::Error(bprotocol::ErrorDescription {
                code: 201,
                description: ByteBuf(b"Some error"),
            }),
        )
        .unwrap();

        let bprotocol::Message {
            transaction_id,
            kind,
            ..
        } = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();

        let mut buf2 = Vec::new();
        bprotocol::serialize_message(&mut buf2, transaction_id, None, None, kind).unwrap();

        if buf.as_slice() != buf2.as_slice() {
            write("error-serialized", buf.as_slice());
            write("error-serialized-again", buf2.as_slice());
            panic!("results don't match, dumped to /tmp/error-serialized-*.bin",)
        }
    }

    #[test]
    fn deserialize_request_find_node() {
        test_deserialize_then_serialize(FIND_NODE_REQUEST, "find_node_request")
    }

    #[test]
    fn deserialize_request_get_peers() {
        test_deserialize_then_serialize(GET_PEERS_REQUEST_0, "get_peers_request_0")
    }

    #[test]
    fn deserialize_response_find_node() {
        test_deserialize_then_serialize(FIND_NODE_RESPONSE_1, "find_node_response")
    }

    #[test]
    fn deserialize_response_find_node_2() {
        test_deserialize_then_serialize(FIND_NODE_RESPONSE_2, "find_node_response_2")
    }

    #[test]
    fn deserialize_response_find_node_3() {
        test_deserialize_then_serialize(FIND_NODE_RESPONSE_3, "find_node_response_3")
    }

    #[test]
    fn deserialize_request_get_peers_request_1() {
        test_deserialize_then_serialize(GET_PEERS_REQUEST_1, "get_peers_request_1")
    }

    #[test]
    fn test_announce() {
        let ann = b"d1:ad2:id20:abcdefghij012345678912:implied_porti1e9:info_hash20:mnopqrstuvwxyz1234564:porti6881e5:token8:aoeusnthe1:q13:announce_peer1:t2:aa1:y1:qe";
        let msg = bprotocol::deserialize_message::<ByteBuf>(ann).unwrap();
        match &msg.kind {
            bprotocol::MessageKind::AnnouncePeer(ann) => {
                dbg!(&ann);
            }
            _ => panic!("wrong kind"),
        }
        let mut buf = Vec::new();
        bprotocol::serialize_message(&mut buf, msg.transaction_id, msg.version, msg.ip, msg.kind)
            .unwrap();
        assert_eq!(ann[..], buf[..]);
    }

    #[test]
    fn deserialize_bencode_packets_captured_from_wireshark() {
        debug_bencode("req: find_node", FIND_NODE_REQUEST);
        debug_bencode("req: get_peers", GET_PEERS_REQUEST_0);
        debug_bencode("resp from the requesting node", FIND_NODE_RESPONSE_1);
        debug_bencode("resp from some random IP", FIND_NODE_RESPONSE_2);
        debug_bencode("another resp from some random IP", FIND_NODE_RESPONSE_3);
        debug_bencode("req to another node", GET_PEERS_REQUEST_1);
    }

    #[test]
    fn serde_want_deserialize() {
        assert_eq!(bencode::from_bytes::<Want>(b"l2:n4e").unwrap(), Want::V4);
        assert_eq!(bencode::from_bytes::<Want>(b"l2:n6e").unwrap(), Want::V6);
        assert_eq!(
            bencode::from_bytes::<Want>(b"l2:n42:n6e").unwrap(),
            Want::Both
        );
        assert_eq!(
            bencode::from_bytes::<Want>(b"l2:aa2:bbe").unwrap(),
            Want::None
        );
    }

    #[test]
    fn serde_want_serialize() {
        let mut w = Vec::new();
        bencode_serialize_to_writer(Want::V6, &mut w).unwrap();
        assert_eq!(&w, b"l2:n6e");

        let mut w = Vec::new();
        bencode_serialize_to_writer(Want::V4, &mut w).unwrap();
        assert_eq!(&w, b"l2:n4e");

        let mut w = Vec::new();
        bencode_serialize_to_writer(Want::Both, &mut w).unwrap();
        assert_eq!(&w, b"l2:n42:n6e");

        let mut w = Vec::new();
        bencode_serialize_to_writer(Want::None, &mut w).unwrap();
        assert_eq!(&w, b"le")
    }

    // -----------------------------------------------------------------------
    // test_serialize_find_node_request
    // -----------------------------------------------------------------------
    #[test]
    fn test_serialize_find_node_request() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new([0xAA; 20]);
        let target = Id20::new([0xBB; 20]);

        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"tx"),
            None,
            None,
            bprotocol::MessageKind::FindNodeRequest(bprotocol::FindNodeRequest {
                id,
                target,
                want: None,
            }),
        )
        .unwrap();

        // Deserialize and verify.
        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        match &msg.kind {
            bprotocol::MessageKind::FindNodeRequest(req) => {
                assert_eq!(req.id, id);
                assert_eq!(req.target, target);
                assert!(req.want.is_none());
            }
            _ => panic!("expected FindNodeRequest"),
        }
        assert_eq!(msg.transaction_id.0, b"tx");
    }

    // -----------------------------------------------------------------------
    // test_serialize_get_peers_request
    // -----------------------------------------------------------------------
    #[test]
    fn test_serialize_get_peers_request() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new([0x11; 20]);
        let info_hash = Id20::new([0x22; 20]);

        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"ab"),
            None,
            None,
            bprotocol::MessageKind::GetPeersRequest(bprotocol::GetPeersRequest {
                id,
                info_hash,
                want: Some(Want::V4),
            }),
        )
        .unwrap();

        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        match &msg.kind {
            bprotocol::MessageKind::GetPeersRequest(req) => {
                assert_eq!(req.id, id);
                assert_eq!(req.info_hash, info_hash);
                assert_eq!(req.want, Some(Want::V4));
            }
            _ => panic!("expected GetPeersRequest"),
        }
    }

    // -----------------------------------------------------------------------
    // test_serialize_announce_peer_request
    // -----------------------------------------------------------------------
    #[test]
    fn test_serialize_announce_peer_request() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new(*b"abcdefghij0123456789");
        let info_hash = Id20::new(*b"mnopqrstuvwxyz123456");

        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"xy"),
            None,
            None,
            bprotocol::MessageKind::AnnouncePeer(bprotocol::AnnouncePeer {
                id,
                implied_port: 1,
                info_hash,
                port: 6881,
                token: ByteBuf(b"tok1"),
            }),
        )
        .unwrap();

        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        match &msg.kind {
            bprotocol::MessageKind::AnnouncePeer(ann) => {
                assert_eq!(ann.id, id);
                assert_eq!(ann.info_hash, info_hash);
                assert_eq!(ann.implied_port, 1);
                assert_eq!(ann.port, 6881);
                assert_eq!(ann.token.0, b"tok1");
            }
            _ => panic!("expected AnnouncePeer"),
        }
    }

    // -----------------------------------------------------------------------
    // test_deserialize_error_response
    // -----------------------------------------------------------------------
    #[test]
    fn test_deserialize_error_response() {
        // Serialize an error, then deserialize it.
        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"er"),
            None,
            None,
            bprotocol::MessageKind::Error(bprotocol::ErrorDescription {
                code: 202,
                description: ByteBuf(b"Server Error"),
            }),
        )
        .unwrap();

        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        match &msg.kind {
            bprotocol::MessageKind::Error(e) => {
                assert_eq!(e.code, 202);
                assert_eq!(e.description.0, b"Server Error");
            }
            _ => panic!("expected Error"),
        }
    }

    // -----------------------------------------------------------------------
    // test_deserialize_malformed_message
    // -----------------------------------------------------------------------
    #[test]
    fn test_deserialize_malformed_message() {
        // Completely invalid bencode.
        let result = bprotocol::deserialize_message::<ByteBuf>(b"not valid bencode");
        assert!(result.is_err());

        // Valid bencode but not a valid DHT message (missing required fields).
        let result = bprotocol::deserialize_message::<ByteBuf>(b"d1:yi1ee");
        assert!(result.is_err());

        // Empty input.
        let result = bprotocol::deserialize_message::<ByteBuf>(b"");
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // test_serialize_deserialize_roundtrip_all_message_types
    // -----------------------------------------------------------------------
    #[test]
    fn test_serialize_deserialize_roundtrip_all_message_types() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new([0x33; 20]);
        let target = Id20::new([0x44; 20]);

        // 1. Ping request
        {
            let mut buf = Vec::new();
            bprotocol::serialize_message(
                &mut buf,
                ByteBuf(b"p1"),
                None,
                None,
                bprotocol::MessageKind::PingRequest(bprotocol::PingRequest { id }),
            )
            .unwrap();
            let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
            assert!(matches!(msg.kind, bprotocol::MessageKind::PingRequest(_)));

            // Re-serialize and check byte equality.
            let mut buf2 = Vec::new();
            bprotocol::serialize_message(
                &mut buf2,
                msg.transaction_id,
                msg.version,
                msg.ip,
                msg.kind,
            )
            .unwrap();
            assert_eq!(buf, buf2, "ping roundtrip mismatch");
        }

        // 2. FindNode request
        {
            let mut buf = Vec::new();
            bprotocol::serialize_message(
                &mut buf,
                ByteBuf(b"f1"),
                None,
                None,
                bprotocol::MessageKind::FindNodeRequest(bprotocol::FindNodeRequest {
                    id,
                    target,
                    want: Some(Want::Both),
                }),
            )
            .unwrap();
            let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
            assert!(matches!(
                msg.kind,
                bprotocol::MessageKind::FindNodeRequest(_)
            ));
            let mut buf2 = Vec::new();
            bprotocol::serialize_message(
                &mut buf2,
                msg.transaction_id,
                msg.version,
                msg.ip,
                msg.kind,
            )
            .unwrap();
            assert_eq!(buf, buf2, "find_node roundtrip mismatch");
        }

        // 3. GetPeers request
        {
            let mut buf = Vec::new();
            bprotocol::serialize_message(
                &mut buf,
                ByteBuf(b"g1"),
                None,
                None,
                bprotocol::MessageKind::GetPeersRequest(bprotocol::GetPeersRequest {
                    id,
                    info_hash: target,
                    want: Some(Want::V6),
                }),
            )
            .unwrap();
            let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
            assert!(matches!(
                msg.kind,
                bprotocol::MessageKind::GetPeersRequest(_)
            ));
            let mut buf2 = Vec::new();
            bprotocol::serialize_message(
                &mut buf2,
                msg.transaction_id,
                msg.version,
                msg.ip,
                msg.kind,
            )
            .unwrap();
            assert_eq!(buf, buf2, "get_peers roundtrip mismatch");
        }

        // 4. Error
        {
            let mut buf = Vec::new();
            bprotocol::serialize_message(
                &mut buf,
                ByteBuf(b"e1"),
                None,
                None,
                bprotocol::MessageKind::Error(bprotocol::ErrorDescription {
                    code: 201,
                    description: ByteBuf(b"A Generic Error Occurred"),
                }),
            )
            .unwrap();
            let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
            assert!(matches!(msg.kind, bprotocol::MessageKind::Error(_)));
            let mut buf2 = Vec::new();
            bprotocol::serialize_message(
                &mut buf2,
                msg.transaction_id,
                msg.version,
                msg.ip,
                msg.kind,
            )
            .unwrap();
            assert_eq!(buf, buf2, "error roundtrip mismatch");
        }

        // 5. Response (with only id, no nodes/values)
        {
            let mut buf = Vec::new();
            bprotocol::serialize_message(
                &mut buf,
                ByteBuf(b"r1"),
                None,
                None,
                bprotocol::MessageKind::Response(bprotocol::Response {
                    id,
                    ..Default::default()
                }),
            )
            .unwrap();
            let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
            assert!(matches!(msg.kind, bprotocol::MessageKind::Response(_)));
            let mut buf2 = Vec::new();
            bprotocol::serialize_message(
                &mut buf2,
                msg.transaction_id,
                msg.version,
                msg.ip,
                msg.kind,
            )
            .unwrap();
            assert_eq!(buf, buf2, "response roundtrip mismatch");
        }

        // 6. AnnouncePeer request
        {
            let mut buf = Vec::new();
            bprotocol::serialize_message(
                &mut buf,
                ByteBuf(b"a1"),
                None,
                None,
                bprotocol::MessageKind::AnnouncePeer(bprotocol::AnnouncePeer {
                    id,
                    implied_port: 0,
                    info_hash: target,
                    port: 8080,
                    token: ByteBuf(b"mytoken"),
                }),
            )
            .unwrap();
            let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
            assert!(matches!(msg.kind, bprotocol::MessageKind::AnnouncePeer(_)));
            let mut buf2 = Vec::new();
            bprotocol::serialize_message(
                &mut buf2,
                msg.transaction_id,
                msg.version,
                msg.ip,
                msg.kind,
            )
            .unwrap();
            assert_eq!(buf, buf2, "announce_peer roundtrip mismatch");
        }
    }

    // -----------------------------------------------------------------------
    // test_serialize_ping_request
    // -----------------------------------------------------------------------
    #[test]
    fn test_serialize_ping_request() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new([0x55; 20]);
        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"pi"),
            None,
            None,
            bprotocol::MessageKind::PingRequest(bprotocol::PingRequest { id }),
        )
        .unwrap();

        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        match &msg.kind {
            bprotocol::MessageKind::PingRequest(req) => {
                assert_eq!(req.id, id);
            }
            _ => panic!("expected PingRequest"),
        }
    }

    // -----------------------------------------------------------------------
    // test_deserialize_unsupported_method
    // -----------------------------------------------------------------------
    #[test]
    fn test_deserialize_unsupported_method() {
        // Manually craft a bencode request with an unknown method name.
        // d1:ad2:id20:aaaaaaaaaaaaaaaaaaaae1:q7:unknown1:t2:xx1:y1:qe
        let raw = b"d1:ad2:id20:aaaaaaaaaaaaaaaaaaaae1:q7:unknown1:t2:xx1:y1:qe";
        let result = bprotocol::deserialize_message::<ByteBuf>(raw);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // test_message_with_version_field
    // -----------------------------------------------------------------------
    #[test]
    fn test_message_with_version_field() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new([0x77; 20]);
        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"v1"),
            Some(ByteBuf(b"rq01")),
            None,
            bprotocol::MessageKind::PingRequest(bprotocol::PingRequest { id }),
        )
        .unwrap();

        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        assert!(msg.version.is_some());
        assert_eq!(msg.version.unwrap().0, b"rq01");
    }

    // -----------------------------------------------------------------------
    // test_error_description_various_codes
    // -----------------------------------------------------------------------
    #[test]
    fn test_error_description_various_codes() {
        // BEP 5 defines error codes:
        // 201 = Generic Error, 202 = Server Error, 203 = Protocol Error, 204 = Method Unknown
        for (code, desc) in [
            (201, "Generic Error"),
            (202, "Server Error"),
            (203, "Protocol Error"),
            (204, "Method Unknown"),
        ] {
            let mut buf = Vec::new();
            bprotocol::serialize_message(
                &mut buf,
                ByteBuf(b"ee"),
                None,
                None,
                bprotocol::MessageKind::Error(bprotocol::ErrorDescription {
                    code,
                    description: ByteBuf(desc.as_bytes()),
                }),
            )
            .unwrap();

            let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
            match &msg.kind {
                bprotocol::MessageKind::Error(e) => {
                    assert_eq!(e.code, code);
                    assert_eq!(e.description.0, desc.as_bytes());
                }
                _ => panic!("expected Error for code {code}"),
            }
        }
    }

    // -----------------------------------------------------------------------
    // BEP 44 message type tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_serialize_bep44_get_request() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new([0x11; 20]);
        let target = Id20::new([0x22; 20]);

        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"g4"),
            None,
            None,
            bprotocol::MessageKind::Bep44GetRequest(bprotocol::Bep44GetRequest {
                id,
                target,
                seq: None,
            }),
        )
        .unwrap();

        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        match &msg.kind {
            bprotocol::MessageKind::Bep44GetRequest(req) => {
                assert_eq!(req.id, id);
                assert_eq!(req.target, target);
                assert!(req.seq.is_none());
            }
            _ => panic!("expected Bep44GetRequest"),
        }
    }

    #[test]
    fn test_serialize_bep44_get_request_with_seq() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new([0x11; 20]);
        let target = Id20::new([0x22; 20]);

        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"g5"),
            None,
            None,
            bprotocol::MessageKind::Bep44GetRequest(bprotocol::Bep44GetRequest {
                id,
                target,
                seq: Some(42),
            }),
        )
        .unwrap();

        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        match &msg.kind {
            bprotocol::MessageKind::Bep44GetRequest(req) => {
                assert_eq!(req.id, id);
                assert_eq!(req.target, target);
                assert_eq!(req.seq, Some(42));
            }
            _ => panic!("expected Bep44GetRequest"),
        }
    }

    #[test]
    fn test_serialize_bep44_put_request() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new([0x33; 20]);

        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"p4"),
            None,
            None,
            bprotocol::MessageKind::Bep44PutRequest(bprotocol::Bep44PutRequest {
                id,
                token: ByteBuf(b"tok123"),
                k: ByteBuf(&[0xAA; 32]),
                sig: ByteBuf(&[0xBB; 64]),
                seq: 7,
                v: ByteBuf(b"5:hello"),
                salt: None,
                cas: None,
            }),
        )
        .unwrap();

        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        match &msg.kind {
            bprotocol::MessageKind::Bep44PutRequest(req) => {
                assert_eq!(req.id, id);
                assert_eq!(req.token.0, b"tok123");
                assert_eq!(req.k.0.len(), 32);
                assert_eq!(req.sig.0.len(), 64);
                assert_eq!(req.seq, 7);
                assert_eq!(req.v.0, b"5:hello");
                assert!(req.salt.is_none());
                assert!(req.cas.is_none());
            }
            _ => panic!("expected Bep44PutRequest"),
        }
    }

    #[test]
    fn test_serialize_bep44_put_request_with_salt_and_cas() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new([0x44; 20]);

        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"p5"),
            None,
            None,
            bprotocol::MessageKind::Bep44PutRequest(bprotocol::Bep44PutRequest {
                id,
                token: ByteBuf(b"token"),
                k: ByteBuf(&[0xCC; 32]),
                sig: ByteBuf(&[0xDD; 64]),
                seq: 10,
                v: ByteBuf(b"i42e"),
                salt: Some(ByteBuf(b"my-salt")),
                cas: Some(9),
            }),
        )
        .unwrap();

        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        match &msg.kind {
            bprotocol::MessageKind::Bep44PutRequest(req) => {
                assert_eq!(req.id, id);
                assert_eq!(req.seq, 10);
                assert_eq!(req.salt.as_ref().unwrap().0, b"my-salt");
                assert_eq!(req.cas, Some(9));
            }
            _ => panic!("expected Bep44PutRequest"),
        }
    }

    #[test]
    fn test_bep44_get_roundtrip() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new([0x55; 20]);
        let target = Id20::new([0x66; 20]);

        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"rt"),
            None,
            None,
            bprotocol::MessageKind::Bep44GetRequest(bprotocol::Bep44GetRequest {
                id,
                target,
                seq: Some(100),
            }),
        )
        .unwrap();

        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        let mut buf2 = Vec::new();
        bprotocol::serialize_message(&mut buf2, msg.transaction_id, msg.version, msg.ip, msg.kind)
            .unwrap();
        assert_eq!(buf, buf2, "BEP 44 get roundtrip mismatch");
    }

    #[test]
    fn test_bep44_put_roundtrip() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new([0x77; 20]);

        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"rp"),
            None,
            None,
            bprotocol::MessageKind::Bep44PutRequest(bprotocol::Bep44PutRequest {
                id,
                token: ByteBuf(b"write-token"),
                k: ByteBuf(&[0xEE; 32]),
                sig: ByteBuf(&[0xFF; 64]),
                seq: 99,
                v: ByteBuf(b"12:Hello World!"),
                salt: Some(ByteBuf(b"salty")),
                cas: Some(98),
            }),
        )
        .unwrap();

        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        let mut buf2 = Vec::new();
        bprotocol::serialize_message(&mut buf2, msg.transaction_id, msg.version, msg.ip, msg.kind)
            .unwrap();
        assert_eq!(buf, buf2, "BEP 44 put roundtrip mismatch");
    }

    #[test]
    fn test_response_with_bep44_fields() {
        use librtbit_core::hash_id::Id20;

        let id = Id20::new([0x88; 20]);

        // A BEP 44 get response includes v, k, sig, seq along with standard fields.
        let mut buf = Vec::new();
        bprotocol::serialize_message(
            &mut buf,
            ByteBuf(b"br"),
            None,
            None,
            bprotocol::MessageKind::Response(bprotocol::Response {
                id,
                token: Some(ByteBuf(b"tok")),
                v: Some(ByteBuf(b"5:hello")),
                k: Some(ByteBuf(&[0xAA; 32])),
                sig: Some(ByteBuf(&[0xBB; 64])),
                seq: Some(42),
                ..Default::default()
            }),
        )
        .unwrap();

        let msg = bprotocol::deserialize_message::<ByteBuf>(&buf).unwrap();
        match &msg.kind {
            bprotocol::MessageKind::Response(resp) => {
                assert_eq!(resp.id, id);
                assert_eq!(resp.v.as_ref().unwrap().0, b"5:hello");
                assert_eq!(resp.k.as_ref().unwrap().0.len(), 32);
                assert_eq!(resp.sig.as_ref().unwrap().0.len(), 64);
                assert_eq!(resp.seq, Some(42));
            }
            _ => panic!("expected Response"),
        }
    }
}
