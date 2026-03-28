use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum BValue {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<BValue>),
    Dict(BTreeMap<Vec<u8>, BValue>),
}

impl BValue {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        encode_into(self, &mut out);
        out
    }
}

fn encode_into(val: &BValue, out: &mut Vec<u8>) {
    match val {
        BValue::Int(n) => {
            out.push(b'i');
            out.extend_from_slice(n.to_string().as_bytes());
            out.push(b'e');
        }
        BValue::Bytes(b) => {
            out.extend_from_slice(b.len().to_string().as_bytes());
            out.push(b':');
            out.extend_from_slice(b);
        }
        BValue::List(items) => {
            out.push(b'l');
            for item in items {
                encode_into(item, out);
            }
            out.push(b'e');
        }
        BValue::Dict(map) => {
            out.push(b'd');
            // BTreeMap is already sorted
            for (k, v) in map {
                out.extend_from_slice(k.len().to_string().as_bytes());
                out.push(b':');
                out.extend_from_slice(k);
                encode_into(v, out);
            }
            out.push(b'e');
        }
    }
}

// Minimal decoder for .torrent files
pub fn decode(data: &[u8]) -> anyhow::Result<(BValue, usize)> {
    if data.is_empty() {
        anyhow::bail!("empty input");
    }
    match data[0] {
        b'i' => {
            let end = data.iter().position(|&b| b == b'e').ok_or_else(|| anyhow::anyhow!("unterminated int"))?;
            let n: i64 = std::str::from_utf8(&data[1..end])?.parse()?;
            Ok((BValue::Int(n), end + 1))
        }
        b'l' => {
            let mut idx = 1;
            let mut items = Vec::new();
            while data[idx] != b'e' {
                let (val, consumed) = decode(&data[idx..])?;
                items.push(val);
                idx += consumed;
            }
            Ok((BValue::List(items), idx + 1))
        }
        b'd' => {
            let mut idx = 1;
            let mut map = BTreeMap::new();
            while data[idx] != b'e' {
                let (key, kc) = decode(&data[idx..])?;
                idx += kc;
                let (val, vc) = decode(&data[idx..])?;
                idx += vc;
                if let BValue::Bytes(k) = key {
                    map.insert(k, val);
                }
            }
            Ok((BValue::Dict(map), idx + 1))
        }
        b'0'..=b'9' => {
            let colon = data.iter().position(|&b| b == b':').ok_or_else(|| anyhow::anyhow!("no colon in string"))?;
            let len: usize = std::str::from_utf8(&data[..colon])?.parse()?;
            let start = colon + 1;
            let end = start + len;
            Ok((BValue::Bytes(data[start..end].to_vec()), end))
        }
        other => anyhow::bail!("unexpected byte: {}", other),
    }
}

pub fn dict_get<'a>(d: &'a BTreeMap<Vec<u8>, BValue>, key: &str) -> Option<&'a BValue> {
    d.get(key.as_bytes())
}

pub fn as_int(v: &BValue) -> Option<i64> {
    match v { BValue::Int(n) => Some(*n), _ => None }
}

pub fn as_bytes(v: &BValue) -> Option<&[u8]> {
    match v { BValue::Bytes(b) => Some(b), _ => None }
}

pub fn as_dict(v: &BValue) -> Option<&BTreeMap<Vec<u8>, BValue>> {
    match v { BValue::Dict(d) => Some(d), _ => None }
}
