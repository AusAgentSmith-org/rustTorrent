#!/usr/bin/env python3
"""
Lightweight mock BitTorrent seeder for benchmarking.

Runs N virtual peers from a single process using asyncio.
Each peer speaks enough of the BT peer wire protocol to:
  1. Complete handshake (info_hash + peer_id)
  2. Send full bitfield
  3. Immediately unchoke
  4. Respond to piece requests by reading from the data file

Usage:
  mock_seeder.py --data-dir /data/testdata \
                 --torrent-dir /data/torrents \
                 --peers 100 \
                 --base-port 6900

Each virtual peer listens on base_port + i.  The tracker is told about
all of them via HTTP announce.
"""

import argparse
import asyncio
import hashlib
import logging
import math
import mmap
import os
import struct
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Dict, List, Optional, Tuple

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [mock] %(message)s",
    stream=sys.stdout,
)
log = logging.getLogger("mock")

# Thread pool for offloading synchronous file I/O from the asyncio event loop
_io_pool = concurrent.futures.ThreadPoolExecutor(max_workers=8)

# ── BitTorrent protocol constants ────────────────────────────────────────────

PSTR = b"BitTorrent protocol"
HANDSHAKE_LEN = 68  # 1 + 19 + 8 + 20 + 20

MSG_CHOKE         = 0
MSG_UNCHOKE       = 1
MSG_INTERESTED    = 2
MSG_NOT_INTERESTED = 3
MSG_HAVE          = 4
MSG_BITFIELD      = 5
MSG_REQUEST       = 6
MSG_PIECE         = 7
MSG_CANCEL        = 8
MSG_KEEPALIVE     = None  # length-prefix = 0


# ── Bencode (minimal, for .torrent parsing) ──────────────────────────────────

def bdecode(data: bytes, idx: int = 0):
    if data[idx:idx+1] == b"i":
        end = data.index(b"e", idx + 1)
        return int(data[idx+1:end]), end + 1
    if data[idx:idx+1] == b"l":
        idx += 1
        result = []
        while data[idx:idx+1] != b"e":
            val, idx = bdecode(data, idx)
            result.append(val)
        return result, idx + 1
    if data[idx:idx+1] == b"d":
        idx += 1
        result = {}
        while data[idx:idx+1] != b"e":
            key, idx = bdecode(data, idx)
            val, idx = bdecode(data, idx)
            result[key] = val
        return result, idx + 1
    # byte string
    colon = data.index(b":", idx)
    length = int(data[idx:colon])
    start = colon + 1
    return data[start:start+length], start + length


def bencode(obj):
    if isinstance(obj, int):
        return f"i{obj}e".encode()
    if isinstance(obj, bytes):
        return str(len(obj)).encode() + b":" + obj
    if isinstance(obj, str):
        return bencode(obj.encode())
    if isinstance(obj, list):
        return b"l" + b"".join(bencode(i) for i in obj) + b"e"
    if isinstance(obj, dict):
        items = sorted(obj.items(), key=lambda x: x[0] if isinstance(x[0], bytes) else x[0].encode())
        return b"d" + b"".join(bencode(k) + bencode(v) for k, v in items) + b"e"
    raise TypeError(f"bencode: unsupported {type(obj)}")


# ── Torrent metadata ─────────────────────────────────────────────────────────

class TorrentMeta:
    """Parsed .torrent metadata needed for seeding."""
    def __init__(self, torrent_path: Path, data_dir: Path):
        raw = torrent_path.read_bytes()
        meta, _ = bdecode(raw)
        info = meta[b"info"]
        self.info_hash = hashlib.sha1(bencode(info)).digest()
        self.info_hash_hex = self.info_hash.hex()
        self.name = info[b"name"].decode(errors="replace")
        self.piece_length = info[b"piece length"]
        self.total_length = info[b"length"]
        self.num_pieces = math.ceil(self.total_length / self.piece_length)
        self.data_path = data_dir / self.name
        self.announce_url = meta.get(b"announce", b"").decode(errors="replace")

        if not self.data_path.exists():
            raise FileNotFoundError(f"Data file missing: {self.data_path}")

        # Keep file handle open to avoid open/close per request
        self._fh = open(self.data_path, "rb")
        self._fh_lock = threading.Lock()

    def bitfield_bytes(self) -> bytes:
        """Full bitfield: all pieces available."""
        n_bytes = math.ceil(self.num_pieces / 8)
        bf = bytearray(n_bytes)
        for i in range(self.num_pieces):
            bf[i // 8] |= (1 << (7 - (i % 8)))
        return bytes(bf)

    def read_block(self, piece: int, offset: int, length: int) -> bytes:
        """Read a block from the data file (thread-safe, reuses file handle)."""
        file_offset = piece * self.piece_length + offset
        with self._fh_lock:
            self._fh.seek(file_offset)
            return self._fh.read(length)


# ── Peer connection handler ──────────────────────────────────────────────────

class PeerSession:
    """Handle one incoming BT peer connection."""

    def __init__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter,
                 torrents: Dict[bytes, TorrentMeta], peer_id: bytes):
        self.reader = reader
        self.writer = writer
        self.torrents = torrents
        self.peer_id = peer_id
        self.torrent: Optional[TorrentMeta] = None
        self.addr = writer.get_extra_info("peername")

    async def run(self):
        try:
            await self._handshake()
            if not self.torrent:
                return
            await self._send_bitfield()
            await self._send_unchoke()
            await self._message_loop()
        except (asyncio.IncompleteReadError, ConnectionResetError, BrokenPipeError,
                ConnectionError, OSError):
            pass
        except Exception as e:
            log.debug("Peer %s error: %s", self.addr, e)
        finally:
            self.writer.close()
            try:
                await self.writer.wait_closed()
            except Exception:
                pass

    async def _handshake(self):
        """Exchange BT handshakes."""
        data = await asyncio.wait_for(self.reader.readexactly(HANDSHAKE_LEN), timeout=10)
        pstrlen = data[0]
        pstr = data[1:1+pstrlen]
        if pstr != PSTR:
            return
        info_hash = data[28:48]
        self.torrent = self.torrents.get(info_hash)
        if not self.torrent:
            return

        # Send our handshake
        resp = bytes([19]) + PSTR + b"\x00" * 8 + info_hash + self.peer_id
        self.writer.write(resp)
        await self.writer.drain()

    async def _send_bitfield(self):
        bf = self.torrent.bitfield_bytes()
        msg = struct.pack("!IB", 1 + len(bf), MSG_BITFIELD) + bf
        self.writer.write(msg)
        await self.writer.drain()

    async def _send_unchoke(self):
        self.writer.write(struct.pack("!IB", 1, MSG_UNCHOKE))
        await self.writer.drain()

    async def _message_loop(self):
        while True:
            length_data = await asyncio.wait_for(self.reader.readexactly(4), timeout=120)
            length = struct.unpack("!I", length_data)[0]
            if length == 0:  # keepalive
                continue
            msg_data = await asyncio.wait_for(self.reader.readexactly(length), timeout=30)
            msg_id = msg_data[0]

            if msg_id == MSG_REQUEST:
                piece, offset, block_len = struct.unpack("!III", msg_data[1:13])
                await self._handle_request(piece, offset, block_len)
            elif msg_id == MSG_INTERESTED:
                await self._send_unchoke()
            elif msg_id == MSG_CANCEL:
                pass  # ignore
            # ignore other messages

    async def _handle_request(self, piece: int, offset: int, length: int):
        """Respond to a piece request (file I/O offloaded to thread pool)."""
        try:
            loop = asyncio.get_running_loop()
            data = await loop.run_in_executor(
                _io_pool, self.torrent.read_block, piece, offset, length
            )
            header = struct.pack("!IBII", 9 + len(data), MSG_PIECE, piece, offset)
            self.writer.write(header + data)
            await self.writer.drain()
        except Exception:
            pass  # silently drop failed reads


# ── Mock seeder service ──────────────────────────────────────────────────────

class MockSeeder:
    """Run N virtual BT peers, each on its own port."""

    def __init__(self, data_dir: Path, torrent_dir: Path,
                 num_peers: int, base_port: int, tracker_url: str):
        self.data_dir = data_dir
        self.torrent_dir = torrent_dir
        self.num_peers = num_peers
        self.base_port = base_port
        self.tracker_url = tracker_url
        self.torrents: Dict[bytes, TorrentMeta] = {}
        self.servers: List[asyncio.Server] = []
        self._peer_ids = [
            f"-MS0001-{i:012d}".encode() for i in range(num_peers)
        ]

    def load_torrents(self):
        """Load all .torrent files from the torrent directory."""
        for p in sorted(self.torrent_dir.glob("*.torrent")):
            try:
                meta = TorrentMeta(p, self.data_dir)
                self.torrents[meta.info_hash] = meta
                log.info("Loaded: %s (%d MB, %d pieces)",
                         meta.name, meta.total_length // (1024*1024), meta.num_pieces)
            except Exception as e:
                log.warning("Skip %s: %s", p.name, e)
        log.info("Loaded %d torrent(s)", len(self.torrents))

    async def start(self):
        """Start all virtual peer listeners."""
        for i in range(self.num_peers):
            port = self.base_port + i
            peer_id = self._peer_ids[i]
            server = await asyncio.start_server(
                lambda r, w, pid=peer_id: self._handle_connection(r, w, pid),
                "0.0.0.0", port,
            )
            self.servers.append(server)

        log.info("Started %d virtual peers on ports %d-%d",
                 self.num_peers, self.base_port,
                 self.base_port + self.num_peers - 1)

        # Announce all peers to tracker
        await self._announce_all()

    async def _handle_connection(self, reader, writer, peer_id):
        session = PeerSession(reader, writer, self.torrents, peer_id)
        await session.run()

    async def _announce_all(self):
        """Announce every (torrent, peer) pair to the tracker."""
        if not self.tracker_url:
            log.info("No tracker URL, skipping announce")
            return

        loop = asyncio.get_running_loop()
        announced = await loop.run_in_executor(_io_pool, self._announce_all_sync)
        log.info("Announced %d (torrent, peer) pairs to tracker", announced)

    def _announce_all_sync(self) -> int:
        """Synchronous announce — run in thread pool."""
        announced = 0
        for meta in self.torrents.values():
            for i in range(self.num_peers):
                port = self.base_port + i
                peer_id = self._peer_ids[i]
                try:
                    ih_encoded = urllib.parse.quote(meta.info_hash, safe="")
                    pid_encoded = urllib.parse.quote(peer_id, safe="")
                    url = (f"{self.tracker_url}?"
                           f"info_hash={ih_encoded}&peer_id={pid_encoded}"
                           f"&port={port}&uploaded=0&downloaded=0&left=0"
                           f"&compact=1&event=started")
                    req = urllib.request.Request(url)
                    urllib.request.urlopen(req, timeout=5)
                    announced += 1
                except Exception as e:
                    if announced == 0:
                        log.warning("Announce failed: %s", e)
        return announced

    async def _re_announce_loop(self):
        """Periodically re-announce to keep peers alive."""
        while True:
            await asyncio.sleep(25)
            await self._announce_all()

    async def run_forever(self):
        """Start and run until killed."""
        self.load_torrents()

        if not self.torrents:
            log.info("No torrents found, waiting for torrents...")
            while not self.torrents:
                await asyncio.sleep(5)
                self.load_torrents()

        await self.start()

        # Re-announce periodically
        asyncio.create_task(self._re_announce_loop())

        # Also watch for new torrents
        asyncio.create_task(self._watch_torrents())

        # Keep running
        await asyncio.Event().wait()

    async def _watch_torrents(self):
        """Watch for new .torrent files and load them."""
        known = set(self.torrents.keys())
        while True:
            await asyncio.sleep(3)
            for p in sorted(self.torrent_dir.glob("*.torrent")):
                try:
                    meta = TorrentMeta(p, self.data_dir)
                    if meta.info_hash not in known:
                        self.torrents[meta.info_hash] = meta
                        known.add(meta.info_hash)
                        log.info("New torrent: %s (%d MB)",
                                 meta.name, meta.total_length // (1024*1024))
                        await self._announce_all()
                except Exception:
                    pass


# ── Health check HTTP endpoint ───────────────────────────────────────────────

async def health_handler(reader, writer):
    await reader.readline()  # read request line
    body = b"OK"
    resp = (
        b"HTTP/1.1 200 OK\r\n"
        b"Content-Length: 2\r\n"
        b"\r\n" + body
    )
    writer.write(resp)
    await writer.drain()
    writer.close()


# ── Main ─────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="Mock BitTorrent seeder")
    parser.add_argument("--data-dir", default="/data/testdata")
    parser.add_argument("--torrent-dir", default="/data/torrents")
    parser.add_argument("--peers", type=int, default=100)
    parser.add_argument("--base-port", type=int, default=6900)
    parser.add_argument("--tracker", default="http://tracker:6969/announce")
    parser.add_argument("--health-port", type=int, default=8080)
    args = parser.parse_args()

    seeder = MockSeeder(
        data_dir=Path(args.data_dir),
        torrent_dir=Path(args.torrent_dir),
        num_peers=args.peers,
        base_port=args.base_port,
        tracker_url=args.tracker,
    )

    async def run():
        # Health check server
        health_srv = await asyncio.start_server(health_handler, "0.0.0.0", args.health_port)
        log.info("Health check on port %d", args.health_port)
        await seeder.run_forever()

    asyncio.run(run())


if __name__ == "__main__":
    main()
