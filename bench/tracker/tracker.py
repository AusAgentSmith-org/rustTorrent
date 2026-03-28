#!/usr/bin/env python3
"""Minimal BitTorrent HTTP tracker for benchmarking.

Handles /announce (BEP 3/23) with compact peer lists.
Stores peers in-memory per info_hash. No persistence needed.
"""

import os
import re
import sys
import time
import struct
import socket
import logging
import threading
import urllib.parse
from http.server import HTTPServer, BaseHTTPRequestHandler
from socketserver import ThreadingMixIn
from collections import defaultdict

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [tracker] %(message)s",
    stream=sys.stdout,
)
log = logging.getLogger("tracker")

# ── State ────────────────────────────────────────────────────────────────────

peers = defaultdict(dict)  # info_hash(bytes) -> {(ip, port): timestamp}
stats = {"announces": 0}
lock = threading.Lock()

ANNOUNCE_INTERVAL = 30
PEER_EXPIRY = 120


def _expire_peers_loop():
    """Background thread: periodically purge stale peers."""
    while True:
        time.sleep(30)
        now = time.time()
        with lock:
            for ih in list(peers.keys()):
                expired = [k for k, v in peers[ih].items() if now - v > PEER_EXPIRY]
                for k in expired:
                    del peers[ih][k]
                if not peers[ih]:
                    del peers[ih]


# ── Bencode ──────────────────────────────────────────────────────────────────

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
        def _key(k):
            return k if isinstance(k, bytes) else k.encode()
        items = sorted(obj.items(), key=lambda x: _key(x[0]))
        return b"d" + b"".join(bencode(k) + bencode(v) for k, v in items) + b"e"
    raise TypeError(f"Cannot bencode {type(obj)}")


# ── Handler ──────────────────────────────────────────────────────────────────

class TrackerHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path == "/announce":
            self.handle_announce()
        elif parsed.path == "/stats":
            self.handle_stats()
        elif parsed.path == "/health":
            self._ok(b"OK")
        else:
            self.send_error(404)

    # ── Announce ─────────────────────────────────────────────────────────

    def handle_announce(self):
        query = self.path.split("?", 1)[1] if "?" in self.path else ""
        params = self._parse_query(query)

        info_hash = params.get(b"info_hash")
        if not info_hash or len(info_hash) != 20:
            return self._tracker_error("Invalid or missing info_hash")

        port = int(params.get(b"port", b"0"))
        event = params.get(b"event", b"").decode("ascii", errors="ignore")
        compact = params.get(b"compact", b"1") == b"1"
        numwant = min(int(params.get(b"numwant", b"200")), 200)
        peer_ip = self.client_address[0]

        with lock:
            stats["announces"] += 1

            if event == "stopped":
                peers[info_hash].pop((peer_ip, port), None)
            else:
                peers[info_hash][(peer_ip, port)] = time.time()

            # Snapshot peer list for this info_hash (no full-table scan)
            peer_list = [
                (ip, p) for (ip, p) in peers[info_hash]
                if not (ip == peer_ip and p == port)
            ][:numwant]
            total = len(peers[info_hash])

        if compact:
            peer_bytes = b""
            for ip, p in peer_list:
                try:
                    peer_bytes += socket.inet_aton(ip) + struct.pack("!H", p)
                except (socket.error, struct.error):
                    continue
            body = bencode({
                b"interval": ANNOUNCE_INTERVAL,
                b"min interval": 10,
                b"complete": total,
                b"incomplete": 0,
                b"peers": peer_bytes,
            })
        else:
            pd = [{b"ip": ip.encode(), b"port": p, b"peer id": b""} for ip, p in peer_list]
            body = bencode({
                b"interval": ANNOUNCE_INTERVAL,
                b"min interval": 10,
                b"complete": total,
                b"incomplete": 0,
                b"peers": pd,
            })

        self._ok(body, content_type="text/plain")

    # ── Stats / health ───────────────────────────────────────────────────

    def handle_stats(self):
        with lock:
            n_torrents = len(peers)
            n_peers = sum(len(p) for p in peers.values())
            body = (
                f"torrents:{n_torrents}\n"
                f"peers:{n_peers}\n"
                f"announces:{stats['announces']}\n"
            ).encode()
        self._ok(body)

    # ── Helpers ──────────────────────────────────────────────────────────

    @staticmethod
    def _parse_query(qs):
        """Parse query string preserving binary info_hash."""
        params = {}
        for part in qs.split("&"):
            if "=" not in part:
                continue
            key, value = part.split("=", 1)
            params[urllib.parse.unquote_to_bytes(key)] = urllib.parse.unquote_to_bytes(value)
        return params

    def _ok(self, body, content_type="text/plain"):
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _tracker_error(self, msg):
        body = bencode({b"failure reason": msg.encode()})
        self._ok(body)

    def log_message(self, fmt, *args):
        if os.environ.get("TRACKER_VERBOSE"):
            log.info(fmt, *args)


class ThreadedHTTPServer(ThreadingMixIn, HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


if __name__ == "__main__":
    port = int(os.environ.get("TRACKER_PORT", "6969"))

    # Start background peer expiry thread
    expiry_thread = threading.Thread(target=_expire_peers_loop, daemon=True)
    expiry_thread.start()

    server = ThreadedHTTPServer(("0.0.0.0", port), TrackerHandler)
    log.info("Listening on 0.0.0.0:%d", port)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        log.info("Shutting down")
        server.shutdown()
