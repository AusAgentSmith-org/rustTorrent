/**
 * Minimal bencode parser for extracting announce URLs from .torrent metadata.
 * Only parses the top-level keys needed: "announce" and "announce-list".
 */

type BencodeValue = string | number | BencodeValue[] | BencodeDict;
type BencodeDict = { [key: string]: BencodeValue };

function decodeString(
  data: Uint8Array,
  offset: number,
): [string, number] | null {
  let i = offset;
  while (i < data.length && data[i] >= 0x30 && data[i] <= 0x39) {
    i++;
  }
  if (i === offset || data[i] !== 0x3a) return null; // ':'
  const len = parseInt(new TextDecoder().decode(data.slice(offset, i)), 10);
  i++; // skip ':'
  if (i + len > data.length) return null;
  const str = new TextDecoder().decode(data.slice(i, i + len));
  return [str, i + len];
}

function decodeInt(data: Uint8Array, offset: number): [number, number] | null {
  if (data[offset] !== 0x69) return null; // 'i'
  let i = offset + 1;
  while (i < data.length && data[i] !== 0x65) {
    i++;
  }
  if (i >= data.length) return null;
  const num = parseInt(new TextDecoder().decode(data.slice(offset + 1, i)), 10);
  return [num, i + 1]; // skip 'e'
}

function decodeValue(
  data: Uint8Array,
  offset: number,
): [BencodeValue, number] | null {
  if (offset >= data.length) return null;
  const ch = data[offset];

  if (ch >= 0x30 && ch <= 0x39) {
    // digit: string
    return decodeString(data, offset);
  }
  if (ch === 0x69) {
    // 'i': integer
    return decodeInt(data, offset);
  }
  if (ch === 0x6c) {
    // 'l': list
    const list: BencodeValue[] = [];
    let i = offset + 1;
    while (i < data.length && data[i] !== 0x65) {
      const result = decodeValue(data, i);
      if (!result) return null;
      list.push(result[0]);
      i = result[1];
    }
    return [list, i + 1]; // skip 'e'
  }
  if (ch === 0x64) {
    // 'd': dictionary
    const dict: BencodeDict = {};
    let i = offset + 1;
    while (i < data.length && data[i] !== 0x65) {
      const keyResult = decodeString(data, i);
      if (!keyResult) return null;
      const [key, nextOffset] = keyResult;
      const valResult = decodeValue(data, nextOffset);
      if (!valResult) return null;
      dict[key] = valResult[0];
      i = valResult[1];
    }
    return [dict, i + 1]; // skip 'e'
  }
  return null;
}

export interface TorrentTrackerInfo {
  announce: string | null;
  announceList: string[][];
}

/**
 * Parse .torrent metadata bytes and extract tracker URLs.
 */
export function extractTrackers(data: Uint8Array): TorrentTrackerInfo {
  const result = decodeValue(data, 0);
  if (!result) return { announce: null, announceList: [] };

  const dict = result[0];
  if (typeof dict !== "object" || Array.isArray(dict))
    return { announce: null, announceList: [] };

  const announce =
    typeof dict["announce"] === "string" ? dict["announce"] : null;

  const announceList: string[][] = [];
  const rawList = dict["announce-list"];
  if (Array.isArray(rawList)) {
    for (const tier of rawList) {
      if (Array.isArray(tier)) {
        const urls = tier.filter((u): u is string => typeof u === "string");
        if (urls.length > 0) announceList.push(urls);
      }
    }
  }

  return { announce, announceList };
}
