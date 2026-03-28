#!/usr/bin/env python3
"""
BitTorrent Client Benchmark — rtbit vs qBittorrent

Orchestrates test-data generation, seeding, downloading, metric collection,
and report generation inside a Docker Compose stack.

Results are written to /results/ (bind-mounted from the host).
"""

import os
import re
import sys
import json
import time
import struct
import socket
import hashlib
import logging
import textwrap
from pathlib import Path
from datetime import datetime
from dataclasses import dataclass, field, asdict
from typing import List, Dict, Optional, Tuple

import requests
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry
import docker
import matplotlib
matplotlib.use("Agg")  # headless backend
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
from matplotlib.gridspec import GridSpec

# ═══════════════════════════════════════════════════════════════════════════════
# Configuration
# ═══════════════════════════════════════════════════════════════════════════════

MB = 1024 * 1024
GB = 1024 * MB

TRACKER_ANNOUNCE = "http://tracker:6969/announce"
TRACKER_STATS = "http://tracker:6969/stats"
TRACKER_HEALTH = "http://tracker:6969/health"
RTBIT_API = "http://rtbit:3030"
QBT_API = "http://qbittorrent:8080"
PROMETHEUS_API = "http://prometheus:9090"

DATA_DIR = Path("/data/testdata")
TORRENT_DIR = Path("/data/torrents")
RESULTS_DIR = Path("/results")

PROJECT_NAME = os.environ.get("COMPOSE_PROJECT_NAME", "bench")
MAX_SEEDERS = int(os.environ.get("MAX_SEEDERS", "10"))
SELECTED_SCENARIOS = os.environ.get("SCENARIOS", "all")

POLL_INTERVAL = 1.0  # seconds between progress polls
METRIC_STEP = "2s"   # Prometheus query resolution

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    stream=sys.stdout,
)
log = logging.getLogger("bench")


# ═══════════════════════════════════════════════════════════════════════════════
# Scenarios
# ═══════════════════════════════════════════════════════════════════════════════

@dataclass
class Scenario:
    name: str
    description: str
    file_size: int           # bytes per file
    num_files: int           # concurrent torrents
    real_seeders: int = 3    # transmission-daemon instances (<=10)
    mock_peers: int   = 0    # lightweight mock peers (0-1000)
    timeout: int      = 300  # max seconds per client
    repetitions: int  = 1    # how many times to run (for averaging)

    @property
    def total_peers(self) -> int:
        return self.real_seeders + self.mock_peers

    @property
    def total_bytes(self) -> int:
        return self.file_size * self.num_files

    # Back-compat: old code used num_seeders
    @property
    def num_seeders(self) -> int:
        return self.real_seeders


# ── Scenario matrix generation ───────────────────────────────────────────────
#
# Axes:
#   total_size:  2, 4, 6, 8, 10, 12, 14, 16, 18, 20 GB
#   num_files:   1, 10, 50, 100
#   peers:       3 real  (baseline)
#                10 real
#                50 mock
#                100 mock
#                250 mock
#                500 mock
#                1000 mock
#
# Each combo = 1 scenario.  The full matrix is 10 x 4 x 7 = 280 scenarios,
# which is too many.  We generate the full set but let the user select
# subsets via SCENARIOS env var (comma-separated names or "all").

def _generate_scenarios() -> List[Scenario]:
    """Build the scenario matrix."""
    scenarios = []

    SIZE_STEPS_GB = [2, 4, 6, 8, 10, 12, 14, 16, 18, 20]
    FILE_COUNTS   = [1, 10, 50, 100]

    # Peer configurations: (real_seeders, mock_peers, label)
    PEER_CONFIGS = [
        (3,   0,    "3p"),
        (10,  0,    "10p"),
        (0,   50,   "50p"),
        (0,   100,  "100p"),
        (0,   250,  "250p"),
        (0,   500,  "500p"),
        (0,   1000, "1000p"),
    ]

    for total_gb in SIZE_STEPS_GB:
        for nf in FILE_COUNTS:
            per_file = (total_gb * GB) // nf
            if per_file < 1 * MB:
                continue  # skip absurdly small per-file sizes

            for real_s, mock_p, peer_label in PEER_CONFIGS:
                total_peers = real_s + mock_p
                name = f"sz{total_gb}g_f{nf}_{peer_label}"
                desc = (f"{total_gb} GB total, {nf} file(s) x "
                        f"{per_file // MB} MB, {total_peers} peers")

                # Timeout scales with data size and inversely with peer count
                base_timeout = max(300, total_gb * 120)
                timeout = base_timeout

                scenarios.append(Scenario(
                    name=name,
                    description=desc,
                    file_size=per_file,
                    num_files=nf,
                    real_seeders=real_s,
                    mock_peers=mock_p,
                    timeout=timeout,
                    repetitions=1,
                ))

    return scenarios


# Pre-built convenience groups for common use
_ALL = _generate_scenarios()

def _pick(names: List[str]) -> List[Scenario]:
    by_name = {s.name: s for s in _ALL}
    return [by_name[n] for n in names if n in by_name]

# Quick smoke test: 2 GB, 1 file, 3 peers (~5 min)
SCENARIOS_QUICK = _pick(["sz2g_f1_3p"])

# Medium: covers all 3 axes at moderate cost (~15-20 min)
#   Size:  2 GB, 4 GB
#   Files: 1, 10, 100
#   Peers: 3 (real), 100 (mock), 500 (mock)
SCENARIOS_MEDIUM = _pick([
    "sz2g_f1_3p",       # baseline: 2 GB, single file, 3 real seeders
    "sz4g_f1_3p",       # size step: 4 GB single file
    "sz2g_f10_3p",      # file count: 10 x 200 MB
    "sz2g_f100_3p",     # file count: 100 x 20 MB
    "sz2g_f1_100p",     # peer scaling: 100 mock peers
    "sz2g_f1_500p",     # peer stress: 500 mock peers
])

# Size ramp with 1 file, 3 real peers (2-20 GB, ~2 hrs)
SCENARIOS_SIZE_RAMP = [s for s in _ALL
                       if s.num_files == 1 and s.real_seeders == 3 and s.mock_peers == 0]

# File count ramp at 10 GB, 3 real peers (~1 hr)
SCENARIOS_FILE_RAMP = [s for s in _ALL
                       if "sz10g" in s.name and s.real_seeders == 3 and s.mock_peers == 0]

# Peer ramp at 10 GB, 1 file, 3-1000 peers (~2 hrs)
SCENARIOS_PEER_RAMP = [s for s in _ALL
                       if "sz10g" in s.name and s.num_files == 1]

# Named groups
SCENARIO_GROUPS = {
    "all":        _ALL,
    "quick":      SCENARIOS_QUICK,
    "medium":     SCENARIOS_MEDIUM,
    "size_ramp":  SCENARIOS_SIZE_RAMP,
    "file_ramp":  SCENARIOS_FILE_RAMP,
    "peer_ramp":  SCENARIOS_PEER_RAMP,
}

def resolve_scenarios(selector: str) -> List[Scenario]:
    """Resolve a scenario selector string to a list of scenarios.

    Accepts: "all", "quick", "size_ramp", "file_ramp", "peer_ramp",
             or comma-separated individual scenario names.
    """
    selector = selector.strip().lower()
    if selector in SCENARIO_GROUPS:
        return SCENARIO_GROUPS[selector]

    names = {s.strip() for s in selector.split(",")}
    matched = [s for s in _ALL if s.name in names]
    if not matched:
        log.error("No scenarios matched: %s", selector)
        log.info("Available groups: %s", ", ".join(SCENARIO_GROUPS.keys()))
        log.info("Example names: %s", ", ".join(s.name for s in _ALL[:10]))
    return matched


# ═══════════════════════════════════════════════════════════════════════════════
# Data classes for results
# ═══════════════════════════════════════════════════════════════════════════════

@dataclass
class MetricSample:
    ts: float
    cpu_pct: float       = 0.0
    mem_bytes: int        = 0
    net_rx_bps: float    = 0.0
    net_tx_bps: float    = 0.0
    disk_read_bps: float = 0.0
    disk_write_bps: float = 0.0
    iowait_pct: float    = 0.0


@dataclass
class ClientResult:
    client: str
    scenario: str
    total_bytes: int           = 0
    duration_sec: float        = 0.0
    avg_speed_mbps: float      = 0.0
    peak_speed_mbps: float     = 0.0
    time_to_first_piece: float = 0.0
    # Aggregated metrics
    cpu_avg: float             = 0.0
    cpu_peak: float            = 0.0
    mem_avg_mb: float          = 0.0
    mem_peak_mb: float         = 0.0
    net_rx_avg_mbps: float     = 0.0
    net_rx_peak_mbps: float    = 0.0
    disk_write_avg_mbps: float = 0.0
    disk_write_peak_mbps: float = 0.0
    iowait_avg: float         = 0.0
    iowait_peak: float        = 0.0
    # Raw time-series (excluded from summary tables, included in JSON)
    timeseries: List[dict] = field(default_factory=list)


# ═══════════════════════════════════════════════════════════════════════════════
# Bencode — minimal encoder for torrent creation
# ═══════════════════════════════════════════════════════════════════════════════

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
        to_b = lambda k: k if isinstance(k, bytes) else k.encode()
        items = sorted(obj.items(), key=lambda x: to_b(x[0]))
        return b"d" + b"".join(bencode(k) + bencode(v) for k, v in items) + b"e"
    raise TypeError(f"Cannot bencode {type(obj)}")


# ═══════════════════════════════════════════════════════════════════════════════
# Test data and torrent generation
# ═══════════════════════════════════════════════════════════════════════════════

def generate_test_file(path: Path, size: int):
    """Write a file of `size` bytes with random content."""
    path.parent.mkdir(parents=True, exist_ok=True)
    chunk_size = 4 * MB
    written = 0
    start = time.time()
    with open(path, "wb") as f:
        while written < size:
            n = min(chunk_size, size - written)
            f.write(os.urandom(n))
            written += n
            # Progress for large files
            if size >= 1 * GB and written % (256 * MB) == 0:
                pct = written * 100 / size
                elapsed = time.time() - start
                rate = written / MB / elapsed if elapsed > 0 else 0
                sys.stdout.write(f"\r    {path.name}: {pct:.0f}% ({rate:.0f} MB/s)")
                sys.stdout.flush()
    elapsed = time.time() - start
    if size >= 1 * GB:
        print()  # newline after progress
    log.info("  Generated %s (%d MB) in %.1fs", path.name, size // MB, elapsed)


def piece_length_for(size: int) -> int:
    if size < 128 * MB:
        return 256 * 1024
    if size < 512 * MB:
        return 512 * 1024
    if size < 2 * GB:
        return 1 * MB
    if size < 8 * GB:
        return 2 * MB
    return 4 * MB  # 20 GB / 4 MB = 5120 pieces


def create_torrent(data_path: Path, tracker_url: str) -> Tuple[bytes, str]:
    """Create a .torrent and return (torrent_bytes, info_hash_hex)."""
    file_size = data_path.stat().st_size
    pl = piece_length_for(file_size)

    pieces = b""
    with open(data_path, "rb") as f:
        while True:
            chunk = f.read(pl)
            if not chunk:
                break
            pieces += hashlib.sha1(chunk).digest()

    info = {
        b"length": file_size,
        b"name": data_path.name.encode(),
        b"piece length": pl,
        b"pieces": pieces,
    }
    info_hash = hashlib.sha1(bencode(info)).hexdigest()

    torrent = {
        b"announce": tracker_url.encode(),
        b"created by": b"bench-orchestrator",
        b"creation date": int(time.time()),
        b"info": info,
    }
    return bencode(torrent), info_hash


def size_label(size: int) -> str:
    if size >= GB:
        return f"{size // GB}gb"
    return f"{size // MB}mb"


# ═══════════════════════════════════════════════════════════════════════════════
# HTTP session with retries
# ═══════════════════════════════════════════════════════════════════════════════

def make_session(retries=3, backoff=0.3) -> requests.Session:
    s = requests.Session()
    adapter = HTTPAdapter(max_retries=Retry(
        total=retries, backoff_factor=backoff,
        status_forcelist=[500, 502, 503, 504],
    ))
    s.mount("http://", adapter)
    return s


# ═══════════════════════════════════════════════════════════════════════════════
# Transmission RPC client  (seeders)
# ═══════════════════════════════════════════════════════════════════════════════

class TransmissionClient:
    def __init__(self, url: str):
        self.url = url
        self.session_id = ""
        self.http = make_session()

    def _rpc(self, method: str, **kwargs):
        payload = {"method": method, "arguments": kwargs}
        headers = {"X-Transmission-Session-Id": self.session_id}
        resp = self.http.post(self.url, json=payload, headers=headers, timeout=30)
        if resp.status_code == 409:
            self.session_id = resp.headers.get("X-Transmission-Session-Id", "")
            headers["X-Transmission-Session-Id"] = self.session_id
            resp = self.http.post(self.url, json=payload, headers=headers, timeout=30)
        resp.raise_for_status()
        result = resp.json()
        if result.get("result") != "success":
            raise RuntimeError(f"Transmission RPC error: {result}")
        return result.get("arguments", {})

    def add_torrent(self, torrent_path: str, download_dir: str = "/data/testdata"):
        return self._rpc("torrent-add", filename=torrent_path, **{"download-dir": download_dir})

    def get_torrents(self, fields=None):
        fields = fields or ["id", "name", "status", "percentDone", "rateUpload"]
        return self._rpc("torrent-get", fields=fields).get("torrents", [])

    def remove_all(self):
        torrents = self.get_torrents(["id"])
        if torrents:
            ids = [t["id"] for t in torrents]
            self._rpc("torrent-remove", ids=ids, **{"delete-local-data": False})

    def is_seeding(self) -> bool:
        """True when all torrents are in seeding state (status=6)."""
        torrents = self.get_torrents(["id", "status", "percentDone"])
        if not torrents:
            return False
        return all(t["status"] == 6 for t in torrents)

    def wait_seeding(self, timeout=120):
        """Block until all torrents are seeding."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.is_seeding():
                return True
            time.sleep(1)
        return False


# ═══════════════════════════════════════════════════════════════════════════════
# rtbit API client
# ═══════════════════════════════════════════════════════════════════════════════

class RtbitClient:
    def __init__(self, url: str):
        self.url = url.rstrip("/")
        self.http = make_session()

    def healthy(self) -> bool:
        try:
            r = self.http.get(f"{self.url}/", timeout=5)
            return r.status_code == 200
        except Exception:
            return False

    def add_torrent(self, torrent_path: Path) -> int:
        """Add a .torrent file, return the torrent ID."""
        data = torrent_path.read_bytes()
        resp = self.http.post(
            f"{self.url}/torrents",
            data=data,
            headers={"Content-Type": "application/octet-stream"},
            params={"overwrite": "true"},
            timeout=60,
        )
        resp.raise_for_status()
        body = resp.json()
        # Response: {"id": N, ...} or {"details": {"id": N, ...}}
        if "id" in body:
            return body["id"]
        if "details" in body and "id" in body["details"]:
            return body["details"]["id"]
        # Fallback: list torrents and find the latest
        torrents = self.list_torrents()
        return max(t["id"] for t in torrents)

    def list_torrents(self) -> list:
        resp = self.http.get(f"{self.url}/torrents", timeout=10)
        resp.raise_for_status()
        data = resp.json()
        # Response is {"torrents": [...], "total": N}
        if isinstance(data, dict) and "torrents" in data:
            return data["torrents"]
        if isinstance(data, list):
            return data
        return []

    def stats(self, torrent_id: int) -> dict:
        resp = self.http.get(f"{self.url}/torrents/{torrent_id}/stats/v1", timeout=10)
        resp.raise_for_status()
        return resp.json()

    def delete_torrent(self, torrent_id):
        try:
            resp = self.http.post(f"{self.url}/torrents/{torrent_id}/delete", timeout=10)
            if resp.status_code >= 400:
                log.warning("  rtbit delete %s: HTTP %d — %s",
                            torrent_id, resp.status_code, resp.text[:200])
        except Exception as e:
            log.warning("  rtbit delete %s failed: %s", torrent_id, e)

    def delete_all(self):
        for t in self.list_torrents():
            tid = t.get("id")
            if tid is not None:
                self.delete_torrent(tid)
        # Also try by info_hash as fallback
        for t in self.list_torrents():
            ih = t.get("info_hash")
            if ih:
                self.delete_torrent(ih)
        # Verify removal
        remaining = self.list_torrents()
        if remaining:
            log.warning("  rtbit: %d torrent(s) still present after delete_all", len(remaining))

    def all_finished(self, ids: List[int]) -> bool:
        return all(self.stats(i).get("finished", False) for i in ids)

    def aggregate_speed(self, ids: List[int]) -> float:
        """Return total download speed in bytes/sec across all torrents."""
        total = 0.0
        for i in ids:
            s = self.stats(i)
            live = s.get("live")
            if live:
                ds = live.get("download_speed", {})
                # rtbit serializes Speed as {"mbps": <float>, "human_readable": "..."}
                # where "mbps" is actually MiB/s (mebibytes per second)
                if isinstance(ds, dict):
                    mibps = ds.get("mbps", 0)
                    total += mibps * MB  # MiB/s -> bytes/sec
                elif isinstance(ds, (int, float)):
                    total += ds
        return total

    def progress_fraction(self, ids: List[int]) -> float:
        """Return average progress as 0.0-1.0."""
        if not ids:
            return 0.0
        fracs = []
        for i in ids:
            s = self.stats(i)
            total = s.get("total_bytes", 1) or 1
            prog = s.get("progress_bytes", 0)
            fracs.append(prog / total)
        return sum(fracs) / len(fracs)


# ═══════════════════════════════════════════════════════════════════════════════
# qBittorrent API client
# ═══════════════════════════════════════════════════════════════════════════════

class QBittorrentClient:
    def __init__(self, url: str):
        self.url = url.rstrip("/")
        self.http = requests.Session()
        self._authenticated = False

    def authenticate(self, docker_client=None):
        """Try several methods to authenticate."""
        # Method 1: try without auth (subnet whitelist)
        try:
            r = self.http.get(f"{self.url}/api/v2/app/version", timeout=5)
            if r.status_code == 200:
                log.info("  qBittorrent: no auth required (subnet whitelist)")
                self._authenticated = True
                return True
        except Exception:
            pass

        # Method 2: default credentials
        if self._try_login("admin", "adminadmin"):
            return True

        # Method 3: parse temporary password from container logs
        if docker_client:
            try:
                containers = docker_client.containers.list(
                    filters={"label": f"com.docker.compose.service=qbittorrent"}
                )
                for c in containers:
                    logs = c.logs(tail=200).decode(errors="replace")
                    m = re.search(r"temporary password.*?:\s*(\S+)", logs, re.IGNORECASE)
                    if m:
                        if self._try_login("admin", m.group(1)):
                            return True
            except Exception as e:
                log.warning("  Could not read qBittorrent logs: %s", e)

        return False

    def _try_login(self, user: str, password: str) -> bool:
        try:
            r = self.http.post(
                f"{self.url}/api/v2/auth/login",
                data={"username": user, "password": password},
                timeout=10,
            )
            if r.text.strip().lower().startswith("ok"):
                log.info("  qBittorrent: logged in as %s", user)
                self._authenticated = True
                return True
        except Exception:
            pass
        return False

    def set_preferences(self, prefs: dict):
        self.http.post(
            f"{self.url}/api/v2/app/setPreferences",
            data={"json": json.dumps(prefs)},
            timeout=10,
        )

    def configure_for_bench(self):
        """Disable DHT/PEX/LSD and other features that interfere with benchmarking."""
        self.set_preferences({
            "dht": False,
            "pex": False,
            "lsd": False,
            "upnp": False,
            "natpmp": False,
            "save_path": "/downloads",
            "temp_path_enabled": False,
            "preallocate_all": False,
            "max_connec": 2000,
            "max_connec_per_torrent": 200,
            "max_uploads": 0,
            "max_uploads_per_torrent": 0,
            "dl_limit": 0,
            "up_limit": 0,
        })
        log.info("  qBittorrent: configured for benchmarking")

    def add_torrent(self, torrent_path: Path):
        with open(torrent_path, "rb") as f:
            self.http.post(
                f"{self.url}/api/v2/torrents/add",
                files={"torrents": (torrent_path.name, f, "application/x-bittorrent")},
                data={"savepath": "/downloads", "sequentialDownload": "false",
                      "skip_checking": "false"},
                timeout=30,
            )

    def get_torrents(self) -> list:
        r = self.http.get(f"{self.url}/api/v2/torrents/info", timeout=10)
        r.raise_for_status()
        return r.json()

    def delete_all(self):
        torrents = self.get_torrents()
        if torrents:
            hashes = "|".join(t["hash"] for t in torrents)
            self.http.post(
                f"{self.url}/api/v2/torrents/delete",
                data={"hashes": hashes, "deleteFiles": "true"},
                timeout=30,
            )

    def all_finished(self) -> bool:
        torrents = self.get_torrents()
        if not torrents:
            return False
        return all(t.get("progress", 0) >= 1.0 for t in torrents)

    def aggregate_speed(self) -> float:
        """Total download speed in bytes/sec."""
        return sum(t.get("dlspeed", 0) for t in self.get_torrents())

    def progress_fraction(self) -> float:
        torrents = self.get_torrents()
        if not torrents:
            return 0.0
        return sum(t.get("progress", 0) for t in torrents) / len(torrents)


# ═══════════════════════════════════════════════════════════════════════════════
# Prometheus metric collector
# ═══════════════════════════════════════════════════════════════════════════════

class MetricsCollector:
    def __init__(self, url: str = PROMETHEUS_API, docker_client=None):
        self.url = url
        self.http = make_session()
        self.docker = docker_client
        # Cache: service_name -> container_id
        self._container_ids: Dict[str, str] = {}

    def _resolve_container_id(self, service: str) -> Optional[str]:
        """Resolve a compose service name to a Docker container ID."""
        if service in self._container_ids:
            return self._container_ids[service]
        if not self.docker:
            return None
        try:
            containers = self.docker.containers.list(
                filters={"label": f"com.docker.compose.service={service}"}
            )
            if containers:
                cid = containers[0].id
                self._container_ids[service] = cid
                return cid
        except Exception as e:
            log.warning("Could not resolve container ID for %s: %s", service, e)
        return None

    def _query_range(self, query: str, start: float, end: float,
                     step: str = METRIC_STEP) -> List[Tuple[float, float]]:
        """Return list of (timestamp, value) from Prometheus."""
        try:
            r = self.http.get(f"{self.url}/api/v1/query_range", params={
                "query": query, "start": start, "end": end, "step": step,
            }, timeout=30)
            r.raise_for_status()
            data = r.json()
            results = data.get("data", {}).get("result", [])
            if not results:
                return []
            # Take the first matching time series
            values = results[0].get("values", [])
            return [(float(ts), float(val)) for ts, val in values]
        except Exception as e:
            log.warning("Prometheus query failed: %s — %s", query[:80], e)
            return []

    def _container_filter(self, service: str) -> str:
        """Build a Prometheus label filter for a container.

        cAdvisor may or may not have Docker labels depending on whether it can
        reach the Docker API.  We try the compose-service label first, then
        fall back to matching by container ID in the cgroup path.
        """
        cid = self._resolve_container_id(service)
        if cid:
            # cAdvisor always exposes the cgroup id which contains the container hash
            return f'id=~".*{cid[:12]}.*"'
        # Fallback: try compose label (works when cAdvisor has Docker access)
        return f'container_label_com_docker_compose_service="{service}"'

    def collect(self, service: str, start: float, end: float) -> List[MetricSample]:
        """Collect all metrics for a service over [start, end]."""
        f = self._container_filter(service)
        # For CPU, match only the 'total' cpu to avoid per-core duplication
        cpu_filter = f'{f},cpu="total"'

        queries = {
            "cpu_pct":        f'rate(container_cpu_usage_seconds_total{{{cpu_filter}}}[10s]) * 100',
            "mem_bytes":      f'container_memory_working_set_bytes{{{f}}}',
            "net_rx_bps":     f'rate(container_network_receive_bytes_total{{{f}}}[10s])',
            "net_tx_bps":     f'rate(container_network_transmit_bytes_total{{{f}}}[10s])',
            "disk_read_bps":  f'rate(container_fs_reads_bytes_total{{{f}}}[10s])',
            "disk_write_bps": f'rate(container_fs_writes_bytes_total{{{f}}}[10s])',
        }
        # Fallback: try blkio for disk I/O if fs metrics are empty
        blkio_queries = {
            "disk_read_bps":  f'rate(container_blkio_device_usage_total{{{f},operation="Read"}}[10s])',
            "disk_write_bps": f'rate(container_blkio_device_usage_total{{{f},operation="Write"}}[10s])',
        }
        # iowait is host-level, not per-container
        queries["iowait_pct"] = 'avg(rate(node_cpu_seconds_total{mode="iowait"}[10s])) * 100'

        raw: Dict[str, List[Tuple[float, float]]] = {}
        for key, q in queries.items():
            raw[key] = self._query_range(q, start, end)

        # Fallback to blkio if fs disk metrics are empty
        if not raw.get("disk_read_bps") and not raw.get("disk_write_bps"):
            for key, q in blkio_queries.items():
                result = self._query_range(q, start, end)
                if result:
                    raw[key] = result

        # Merge into time-aligned samples
        all_ts = sorted({ts for series in raw.values() for ts, _ in series})
        if not all_ts:
            return []

        def _lookup(series, ts):
            for t, v in series:
                if abs(t - ts) < 3:
                    return v
            return 0.0

        samples = []
        for ts in all_ts:
            samples.append(MetricSample(
                ts=ts,
                cpu_pct=_lookup(raw.get("cpu_pct", []), ts),
                mem_bytes=int(_lookup(raw.get("mem_bytes", []), ts)),
                net_rx_bps=_lookup(raw.get("net_rx_bps", []), ts),
                net_tx_bps=_lookup(raw.get("net_tx_bps", []), ts),
                disk_read_bps=_lookup(raw.get("disk_read_bps", []), ts),
                disk_write_bps=_lookup(raw.get("disk_write_bps", []), ts),
                iowait_pct=_lookup(raw.get("iowait_pct", []), ts),
            ))
        return samples


# ═══════════════════════════════════════════════════════════════════════════════
# Seeder manager — discovers and controls transmission instances
# ═══════════════════════════════════════════════════════════════════════════════

class SeederManager:
    def __init__(self, docker_client):
        self.docker = docker_client
        self._clients: List[TransmissionClient] = []

    def discover(self):
        """Find all seeder containers and create RPC clients."""
        containers = self.docker.containers.list(
            filters={"label": f"com.docker.compose.service=seeder"}
        )
        self._clients = []
        for c in containers:
            networks = c.attrs["NetworkSettings"]["Networks"]
            for net_name, net_cfg in networks.items():
                ip = net_cfg.get("IPAddress")
                if ip:
                    self._clients.append(
                        TransmissionClient(f"http://{ip}:9091/transmission/rpc")
                    )
                    break
        log.info("Discovered %d seeder(s)", len(self._clients))

    @property
    def count(self) -> int:
        return len(self._clients)

    def add_torrent_to(self, n: int, torrent_path: str):
        """Add a torrent to the first N seeders."""
        for client in self._clients[:n]:
            client.add_torrent(torrent_path)

    def wait_all_seeding(self, n: int, timeout: int = 180):
        """Wait until the first N seeders are in seeding state."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            ready = sum(1 for c in self._clients[:n] if c.is_seeding())
            if ready >= n:
                return True
            time.sleep(1)
        log.warning("Only %d/%d seeders ready after %ds", ready, n, timeout)
        return False

    def remove_all(self):
        """Remove all torrents from all seeders."""
        for c in self._clients:
            try:
                c.remove_all()
            except Exception:
                pass


# ═══════════════════════════════════════════════════════════════════════════════
# Benchmark runner
# ═══════════════════════════════════════════════════════════════════════════════

class BenchmarkRunner:
    def __init__(self):
        self.docker = docker.from_env()
        self.rtbit = RtbitClient(RTBIT_API)
        self.qbt = QBittorrentClient(QBT_API)
        self.seeders = SeederManager(self.docker)
        self.metrics = MetricsCollector(docker_client=self.docker)
        self.all_results: List[Tuple[ClientResult, ClientResult]] = []

        DATA_DIR.mkdir(parents=True, exist_ok=True)
        TORRENT_DIR.mkdir(parents=True, exist_ok=True)
        RESULTS_DIR.mkdir(parents=True, exist_ok=True)

    # ── Service readiness ────────────────────────────────────────────────

    def wait_for_services(self, timeout=300):
        log.info("Waiting for services...")
        services = [
            ("tracker", TRACKER_HEALTH),
            ("rtbit",   f"{RTBIT_API}/"),
            ("qBittorrent", f"{QBT_API}/"),
            ("prometheus",  f"{PROMETHEUS_API}/-/healthy"),
        ]
        http = make_session(retries=1, backoff=0.1)
        for name, url in services:
            deadline = time.time() + timeout
            while time.time() < deadline:
                try:
                    r = http.get(url, timeout=5)
                    if r.status_code < 500:
                        log.info("  %s: ready", name)
                        break
                except Exception:
                    pass
                time.sleep(2)
            else:
                raise TimeoutError(f"{name} not ready after {timeout}s")

        # Discover seeders (retry until at least 1 found)
        deadline = time.time() + 120
        while time.time() < deadline:
            self.seeders.discover()
            if self.seeders.count > 0:
                break
            time.sleep(3)

        # Verify seeder RPC works
        ok = 0
        for c in self.seeders._clients:
            try:
                c.get_torrents()
                ok += 1
            except Exception:
                pass
        log.info("  Seeders: %d/%d RPC-ready", ok, self.seeders.count)

    # ── qBittorrent setup ────────────────────────────────────────────────

    def setup_qbittorrent(self):
        log.info("Configuring qBittorrent...")
        if not self.qbt.authenticate(self.docker):
            raise RuntimeError("Cannot authenticate to qBittorrent")
        self.qbt.configure_for_bench()

    # ── Data preparation ─────────────────────────────────────────────────

    def prepare_data(self, scenarios: List[Scenario]):
        """Pre-generate all test data files and torrents needed."""
        log.info("Preparing test data...")
        # Determine unique (size, index) pairs needed
        needed: Dict[int, int] = {}  # size -> max count
        for sc in scenarios:
            needed[sc.file_size] = max(needed.get(sc.file_size, 0), sc.num_files)

        total_gen = sum(sz * cnt for sz, cnt in needed.items())
        log.info("  Need %d unique file size(s), up to %.1f GB total",
                 len(needed), total_gen / GB)

        for size, count in sorted(needed.items()):
            label = size_label(size)
            log.info("  %s: %d file(s) of %d MB each", label, count, size // MB)
            for i in range(count):
                fname = f"bench_{label}_{i:03d}.bin"
                fpath = DATA_DIR / fname
                tname = f"bench_{label}_{i:03d}.torrent"
                tpath = TORRENT_DIR / tname

                if not fpath.exists():
                    log.info("  Generating %s (%s)...", fname, label)
                    generate_test_file(fpath, size)
                else:
                    # Verify size matches
                    actual = fpath.stat().st_size
                    if actual != size:
                        log.warning("  %s: expected %d, got %d — regenerating",
                                    fname, size, actual)
                        generate_test_file(fpath, size)

                if not tpath.exists():
                    log.info("  Creating %s...", tname)
                    torrent_bytes, info_hash = create_torrent(fpath, TRACKER_ANNOUNCE)
                    tpath.write_bytes(torrent_bytes)
                    log.info("    info_hash: %s", info_hash)

        log.info("Test data ready.")

    def torrent_paths(self, sc: Scenario) -> List[Path]:
        """Return torrent file paths for a scenario."""
        label = size_label(sc.file_size)
        return [TORRENT_DIR / f"bench_{label}_{i:03d}.torrent" for i in range(sc.num_files)]

    # ── Run a scenario for one client ────────────────────────────────────

    def _run_client(self, client_name: str, sc: Scenario,
                    torrent_paths: List[Path]) -> ClientResult:
        result = ClientResult(client=client_name, scenario=sc.name,
                              total_bytes=sc.file_size * sc.num_files)

        log.info("  [%s] Adding %d torrent(s)...", client_name, len(torrent_paths))

        if client_name == "rtbit":
            ids = []
            for tp in torrent_paths:
                try:
                    tid = self.rtbit.add_torrent(tp)
                    ids.append(tid)
                except Exception as e:
                    log.error("  [rtbit] Failed to add %s: %s", tp.name, e)
        else:
            for tp in torrent_paths:
                try:
                    self.qbt.add_torrent(tp)
                except Exception as e:
                    log.error("  [qbt] Failed to add %s: %s", tp.name, e)
            # qBittorrent doesn't return IDs easily; we track by hash

        # ── Monitor download progress ────────────────────────────────────
        start_time = time.time()
        first_piece_time = None
        peak_speed = 0.0
        speeds = []

        log.info("  [%s] Downloading (timeout %ds)...", client_name, sc.timeout)

        deadline = start_time + sc.timeout
        while time.time() < deadline:
            try:
                if client_name == "rtbit":
                    finished = self.rtbit.all_finished(ids) if ids else False
                    progress = self.rtbit.progress_fraction(ids) if ids else 0
                    speed = self.rtbit.aggregate_speed(ids) if ids else 0
                else:
                    finished = self.qbt.all_finished()
                    progress = self.qbt.progress_fraction()
                    speed = self.qbt.aggregate_speed()

                if progress > 0.001 and first_piece_time is None:
                    first_piece_time = time.time() - start_time

                peak_speed = max(peak_speed, speed)
                if speed > 0:
                    speeds.append(speed)

                if finished:
                    log.info("  [%s] Complete!", client_name)
                    break

                # Progress bar
                bar_len = 30
                filled = int(bar_len * progress)
                bar = "#" * filled + "-" * (bar_len - filled)
                speed_mb = speed / MB if speed else 0
                sys.stdout.write(
                    f"\r  [{client_name}] [{bar}] {progress*100:5.1f}% "
                    f"@ {speed_mb:.1f} MB/s"
                )
                sys.stdout.flush()

            except Exception as e:
                log.debug("  [%s] Poll error: %s", client_name, e)

            time.sleep(POLL_INTERVAL)
        else:
            log.warning("  [%s] TIMEOUT after %ds", client_name, sc.timeout)

        print()  # newline after progress bar
        end_time = time.time()

        # ── Populate result ──────────────────────────────────────────────
        result.duration_sec = end_time - start_time
        result.time_to_first_piece = first_piece_time or result.duration_sec
        result.peak_speed_mbps = (peak_speed * 8) / (1000 * 1000)  # bits/sec -> Mbps
        if speeds:
            avg_speed = sum(speeds) / len(speeds)
            result.avg_speed_mbps = (avg_speed * 8) / (1000 * 1000)
        elif result.duration_sec > 0:
            result.avg_speed_mbps = (result.total_bytes * 8) / (result.duration_sec * 1000 * 1000)

        # ── Collect Prometheus metrics ───────────────────────────────────
        log.info("  [%s] Collecting metrics from Prometheus...", client_name)
        # Allow a few seconds for metrics to be scraped
        time.sleep(3)

        samples = self.metrics.collect(client_name, start_time, end_time)
        if samples:
            cpus = [s.cpu_pct for s in samples]
            mems = [s.mem_bytes for s in samples]
            rxs = [s.net_rx_bps for s in samples]
            dws = [s.disk_write_bps for s in samples]
            iow = [s.iowait_pct for s in samples]

            result.cpu_avg = sum(cpus) / len(cpus) if cpus else 0
            result.cpu_peak = max(cpus) if cpus else 0
            result.mem_avg_mb = (sum(mems) / len(mems)) / MB if mems else 0
            result.mem_peak_mb = max(mems) / MB if mems else 0
            result.net_rx_avg_mbps = (sum(rxs) / len(rxs)) * 8 / 1e6 if rxs else 0
            result.net_rx_peak_mbps = max(rxs) * 8 / 1e6 if rxs else 0
            result.disk_write_avg_mbps = (sum(dws) / len(dws)) / MB if dws else 0
            result.disk_write_peak_mbps = max(dws) / MB if dws else 0
            result.iowait_avg = sum(iow) / len(iow) if iow else 0
            result.iowait_peak = max(iow) if iow else 0

            result.timeseries = [asdict(s) for s in samples]

        log.info("  [%s] Done: %.1fs, avg %.1f Mbps", client_name,
                 result.duration_sec, result.avg_speed_mbps)
        return result

    # ── Cleanup between runs ─────────────────────────────────────────────

    def cleanup_client(self, client_name: str):
        log.info("  [%s] Cleaning up...", client_name)
        try:
            if client_name == "rtbit":
                self.rtbit.delete_all()
            else:
                self.qbt.delete_all()
        except Exception as e:
            log.warning("  Cleanup error: %s", e)
        # Wait for file handles to be released and torrent to be fully removed
        time.sleep(3)
        # Verify cleanup
        try:
            if client_name == "rtbit":
                remaining = self.rtbit.list_torrents()
                if remaining:
                    log.warning("  [%s] Still has %d torrent(s) after cleanup!",
                                client_name, len(remaining))
                    # Force retry
                    self.rtbit.delete_all()
                    time.sleep(2)
        except Exception:
            pass

    # ── Run a full scenario ──────────────────────────────────────────────

    def run_scenario(self, sc: Scenario) -> Tuple[ClientResult, ClientResult]:
        log.info("=" * 60)
        log.info("SCENARIO: %s — %s", sc.name, sc.description)
        log.info("  Files: %d x %s, Real seeders: %d, Mock peers: %d",
                 sc.num_files, size_label(sc.file_size),
                 sc.real_seeders, sc.mock_peers)
        log.info("=" * 60)

        tpaths = self.torrent_paths(sc)

        # ── Setup real seeders (transmission) ────────────────────────────
        if sc.real_seeders > 0:
            log.info("Setting up %d real seeder(s)...", sc.real_seeders)
            self.seeders.remove_all()
            time.sleep(1)
            for tp in tpaths:
                self.seeders.add_torrent_to(sc.real_seeders, str(tp))

            # Scale timeout for seeder verification with file size
            verify_timeout = max(180, sc.file_size * sc.num_files // (50 * MB))
            log.info("Waiting for seeders to verify (timeout %ds)...", verify_timeout)
            if not self.seeders.wait_all_seeding(sc.real_seeders, timeout=verify_timeout):
                log.error("Seeders not ready — skipping scenario")
                empty = ClientResult(client="rtbit", scenario=sc.name)
                return (empty, ClientResult(client="qbittorrent", scenario=sc.name))

        # ── Mock seeder is always running and auto-discovers torrents ────
        if sc.mock_peers > 0:
            log.info("Mock seeder active with %d peers (auto-discovers torrents)",
                     sc.mock_peers)
            # Give mock seeder a moment to discover and announce
            time.sleep(5)

        log.info("Seeding ready (%d real + %d mock = %d total peers). "
                 "Running downloads sequentially.",
                 sc.real_seeders, sc.mock_peers, sc.total_peers)

        # Run rtbit
        rtbit_result = self._run_client("rtbit", sc, tpaths)
        self.cleanup_client("rtbit")

        # Brief cooldown between clients
        time.sleep(3)

        # Run qBittorrent
        qbt_result = self._run_client("qbittorrent", sc, tpaths)
        self.cleanup_client("qbittorrent")

        # Clean up seeders
        if sc.real_seeders > 0:
            self.seeders.remove_all()
        time.sleep(2)

        return (rtbit_result, qbt_result)

    # ── Run all scenarios ────────────────────────────────────────────────

    def run(self):
        log.info("=" * 60)
        log.info("  BitTorrent Client Benchmark: rtbit vs qBittorrent")
        log.info("=" * 60)

        self.wait_for_services()
        self.setup_qbittorrent()

        # Select scenarios
        scenarios = resolve_scenarios(SELECTED_SCENARIOS)
        if not scenarios:
            return

        # Summary
        total_data = sum(s.total_bytes for s in scenarios)
        total_runs = sum(s.repetitions for s in scenarios)
        log.info("Running %d scenario(s) (%d total runs, %.1f GB data to generate):",
                 len(scenarios), total_runs, total_data / GB)
        for s in scenarios:
            log.info("  %-30s  %4d MB x %3d files = %5d MB  %4d peers  timeout=%ds",
                     s.name, s.file_size // MB, s.num_files,
                     s.total_bytes // MB, s.total_peers, s.timeout)

        self.prepare_data(scenarios)

        for sc in scenarios:
            for rep in range(sc.repetitions):
                rep_label = f" (rep {rep+1}/{sc.repetitions})" if sc.repetitions > 1 else ""
                try:
                    log.info("--- %s%s ---", sc.name, rep_label)
                    rtbit_res, qbt_res = self.run_scenario(sc)
                    # Tag repetition in result
                    if sc.repetitions > 1:
                        rtbit_res.scenario = f"{sc.name}_r{rep+1}"
                        qbt_res.scenario = f"{sc.name}_r{rep+1}"
                    self.all_results.append((rtbit_res, qbt_res))
                except Exception as e:
                    log.error("Scenario %s%s failed: %s", sc.name, rep_label,
                              e, exc_info=True)

        self.generate_report()

    # ── Report ───────────────────────────────────────────────────────────

    def generate_report(self):
        if not self.all_results:
            log.warning("No results to report.")
            return

        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")

        # ── JSON (full data including time series) ───────────────────────
        json_path = RESULTS_DIR / f"benchmark_{timestamp}.json"
        json_data = []
        for rq, qb in self.all_results:
            json_data.append({
                "scenario": rq.scenario,
                "rtbit": asdict(rq),
                "qbittorrent": asdict(qb),
            })
        json_path.write_text(json.dumps(json_data, indent=2, default=str))
        log.info("Full results: %s", json_path)

        # ── Summary table (console + text file) ─────────────────────────
        summary_lines = self._build_summary_table()
        summary_text = "\n".join(summary_lines)
        print("\n" + summary_text)

        summary_path = RESULTS_DIR / f"summary_{timestamp}.txt"
        summary_path.write_text(summary_text)
        log.info("Summary: %s", summary_path)

        # ── CSV (for easy import into sheets/plotting tools) ─────────────
        csv_path = RESULTS_DIR / f"benchmark_{timestamp}.csv"
        self._write_csv(csv_path)
        log.info("CSV: %s", csv_path)

        # ── Charts ──────────────────────────────────────────────────────
        charts_dir = RESULTS_DIR / f"charts_{timestamp}"
        charts_dir.mkdir(exist_ok=True)
        self._generate_charts(charts_dir)
        log.info("Charts: %s/", charts_dir)

    def _build_summary_table(self) -> List[str]:
        lines = []
        lines.append("")
        lines.append("=" * 78)
        lines.append("  BENCHMARK RESULTS: rtbit vs qBittorrent")
        lines.append("=" * 78)

        for rq, qb in self.all_results:
            lines.append("")
            lines.append(f"  Scenario: {rq.scenario}")
            lines.append("-" * 78)
            lines.append(f"  {'Metric':<24} {'rtbit':>15} {'qBittorrent':>15} {'Delta':>12}")
            lines.append("-" * 78)

            metrics = [
                ("Duration",         f"{rq.duration_sec:.1f}s",          f"{qb.duration_sec:.1f}s",           rq.duration_sec, qb.duration_sec, True),
                ("Avg Speed",        f"{rq.avg_speed_mbps:.1f} Mbps",    f"{qb.avg_speed_mbps:.1f} Mbps",     rq.avg_speed_mbps, qb.avg_speed_mbps, False),
                ("Peak Speed",       f"{rq.peak_speed_mbps:.1f} Mbps",   f"{qb.peak_speed_mbps:.1f} Mbps",    rq.peak_speed_mbps, qb.peak_speed_mbps, False),
                ("Time to 1st Piece", f"{rq.time_to_first_piece:.2f}s",  f"{qb.time_to_first_piece:.2f}s",    rq.time_to_first_piece, qb.time_to_first_piece, True),
                ("CPU Avg",          f"{rq.cpu_avg:.1f}%",               f"{qb.cpu_avg:.1f}%",                rq.cpu_avg, qb.cpu_avg, True),
                ("CPU Peak",         f"{rq.cpu_peak:.1f}%",              f"{qb.cpu_peak:.1f}%",               rq.cpu_peak, qb.cpu_peak, True),
                ("Memory Avg",       f"{rq.mem_avg_mb:.1f} MB",          f"{qb.mem_avg_mb:.1f} MB",           rq.mem_avg_mb, qb.mem_avg_mb, True),
                ("Memory Peak",      f"{rq.mem_peak_mb:.1f} MB",         f"{qb.mem_peak_mb:.1f} MB",          rq.mem_peak_mb, qb.mem_peak_mb, True),
                ("Net RX Avg",       f"{rq.net_rx_avg_mbps:.1f} Mbps",   f"{qb.net_rx_avg_mbps:.1f} Mbps",    rq.net_rx_avg_mbps, qb.net_rx_avg_mbps, False),
                ("Net RX Peak",      f"{rq.net_rx_peak_mbps:.1f} Mbps",  f"{qb.net_rx_peak_mbps:.1f} Mbps",   rq.net_rx_peak_mbps, qb.net_rx_peak_mbps, False),
                ("Disk Write Avg",   f"{rq.disk_write_avg_mbps:.1f} MB/s", f"{qb.disk_write_avg_mbps:.1f} MB/s", rq.disk_write_avg_mbps, qb.disk_write_avg_mbps, False),
                ("Disk Write Peak",  f"{rq.disk_write_peak_mbps:.1f} MB/s", f"{qb.disk_write_peak_mbps:.1f} MB/s", rq.disk_write_peak_mbps, qb.disk_write_peak_mbps, False),
                ("IO Wait Avg",      f"{rq.iowait_avg:.2f}%",            f"{qb.iowait_avg:.2f}%",             rq.iowait_avg, qb.iowait_avg, True),
                ("IO Wait Peak",     f"{rq.iowait_peak:.2f}%",           f"{qb.iowait_peak:.2f}%",            rq.iowait_peak, qb.iowait_peak, True),
            ]

            for label, rq_str, qb_str, rq_val, qb_val, lower_is_better in metrics:
                delta = self._delta_str(rq_val, qb_val, lower_is_better)
                lines.append(f"  {label:<24} {rq_str:>15} {qb_str:>15} {delta:>12}")

            lines.append("-" * 78)

        lines.append("")
        lines.append("  Lower-is-better metrics: Duration, Time to 1st Piece, CPU, Memory, IO Wait")
        lines.append("  Higher-is-better metrics: Speed, Net RX, Disk Write")
        lines.append("  Delta shows rtbit advantage: positive = rtbit wins")
        lines.append("")
        return lines

    @staticmethod
    def _delta_str(rq_val: float, qb_val: float, lower_is_better: bool) -> str:
        if qb_val == 0 and rq_val == 0:
            return "—"
        if qb_val == 0:
            return "—"
        pct = ((qb_val - rq_val) / qb_val) * 100
        if lower_is_better:
            # Lower rtbit = positive delta (rtbit wins)
            prefix = "+" if pct > 0 else ""
        else:
            # Higher rtbit = positive delta, so invert
            pct = -pct
            prefix = "+" if pct > 0 else ""
        return f"{prefix}{pct:.1f}%"

    # ── Chart generation ────────────────────────────────────────────────

    COLORS = {"rtbit": "#E45932", "qbittorrent": "#2681FF"}

    def _generate_charts(self, charts_dir: Path):
        """Generate all charts from benchmark results."""
        log.info("Generating charts...")
        plt.rcParams.update({
            "figure.facecolor": "#1a1a2e",
            "axes.facecolor": "#16213e",
            "axes.edgecolor": "#555",
            "axes.labelcolor": "#ccc",
            "text.color": "#ddd",
            "xtick.color": "#aaa",
            "ytick.color": "#aaa",
            "grid.color": "#333",
            "grid.alpha": 0.5,
            "legend.facecolor": "#16213e",
            "legend.edgecolor": "#555",
            "font.size": 11,
            "axes.titlesize": 14,
            "axes.labelsize": 12,
        })

        for rq, qb in self.all_results:
            scenario = rq.scenario
            # Per-scenario charts
            self._chart_timeseries(charts_dir, scenario, rq, qb)
            self._chart_bar_comparison(charts_dir, scenario, rq, qb)

        # Cross-scenario summary charts (only if multiple scenarios)
        if len(self.all_results) > 1:
            self._chart_scenario_comparison(charts_dir)
            self._chart_resource_efficiency(charts_dir)

        # Always generate the overview dashboard
        self._chart_dashboard(charts_dir)

        plt.close("all")
        log.info("  Generated %d chart(s)", len(list(charts_dir.glob("*.png"))))

    def _ts_to_relative(self, timeseries: List[dict]) -> List[float]:
        """Convert absolute timestamps to relative seconds from start."""
        if not timeseries:
            return []
        t0 = timeseries[0]["ts"]
        return [p["ts"] - t0 for p in timeseries]

    def _chart_timeseries(self, charts_dir: Path, scenario: str,
                          rq: 'ClientResult', qb: 'ClientResult'):
        """Time-series plots: CPU, Memory, IO Wait over time for both clients."""
        metrics_config = [
            ("cpu_pct",      "CPU Usage (%)",       "%",    1.0),
            ("mem_bytes",    "Memory (MB)",          "MB",   1 / MB),
            ("iowait_pct",   "IO Wait (%)",         "%",    1.0),
            ("net_rx_bps",   "Network RX (MB/s)",   "MB/s", 1 / MB),
            ("disk_write_bps", "Disk Write (MB/s)", "MB/s", 1 / MB),
        ]

        # Filter to metrics that have non-zero data
        active_metrics = []
        for key, label, unit, scale in metrics_config:
            has_data = False
            for ts in [rq.timeseries, qb.timeseries]:
                if any(p.get(key, 0) != 0 for p in ts):
                    has_data = True
                    break
            if has_data:
                active_metrics.append((key, label, unit, scale))

        if not active_metrics:
            return

        n = len(active_metrics)
        fig, axes = plt.subplots(n, 1, figsize=(14, 3.5 * n), sharex=False)
        if n == 1:
            axes = [axes]

        fig.suptitle(f"Time-Series Metrics — {scenario}", fontsize=16, y=0.98)

        for ax, (key, label, unit, scale) in zip(axes, active_metrics):
            for result, name in [(rq, "rtbit"), (qb, "qbittorrent")]:
                ts = result.timeseries
                if not ts:
                    continue
                t = self._ts_to_relative(ts)
                vals = [p.get(key, 0) * scale for p in ts]
                ax.plot(t, vals, label=name, color=self.COLORS[name],
                        linewidth=2, alpha=0.9)
                ax.fill_between(t, vals, alpha=0.15, color=self.COLORS[name])

            ax.set_ylabel(unit)
            ax.set_title(label, fontsize=12, pad=6)
            ax.legend(loc="upper right", framealpha=0.8)
            ax.grid(True, alpha=0.3)
            ax.set_xlim(left=0)
            ax.set_ylim(bottom=0)

        axes[-1].set_xlabel("Time (seconds)")
        fig.tight_layout(rect=[0, 0, 1, 0.96])
        fig.savefig(charts_dir / f"{scenario}_timeseries.png", dpi=150,
                    bbox_inches="tight")
        plt.close(fig)

    def _chart_bar_comparison(self, charts_dir: Path, scenario: str,
                              rq: 'ClientResult', qb: 'ClientResult'):
        """Side-by-side bar chart comparing all metrics for a scenario."""
        categories = [
            ("Duration\n(sec)",       rq.duration_sec,         qb.duration_sec),
            ("Avg Speed\n(Mbps)",     rq.avg_speed_mbps,       qb.avg_speed_mbps),
            ("Peak Speed\n(Mbps)",    rq.peak_speed_mbps,      qb.peak_speed_mbps),
            ("1st Piece\n(sec)",      rq.time_to_first_piece,  qb.time_to_first_piece),
            ("CPU Avg\n(%)",          rq.cpu_avg,              qb.cpu_avg),
            ("CPU Peak\n(%)",         rq.cpu_peak,             qb.cpu_peak),
            ("Mem Avg\n(MB)",         rq.mem_avg_mb,           qb.mem_avg_mb),
            ("Mem Peak\n(MB)",        rq.mem_peak_mb,          qb.mem_peak_mb),
            ("IO Wait\nAvg (%)",      rq.iowait_avg,           qb.iowait_avg),
        ]
        # Filter out zero-value pairs
        categories = [(l, r, q) for l, r, q in categories if r != 0 or q != 0]
        if not categories:
            return

        labels = [c[0] for c in categories]
        rq_vals = [c[1] for c in categories]
        qb_vals = [c[2] for c in categories]

        x = range(len(labels))
        width = 0.35

        fig, ax = plt.subplots(figsize=(max(10, len(labels) * 1.5), 6))
        bars1 = ax.bar([i - width / 2 for i in x], rq_vals, width,
                       label="rtbit", color=self.COLORS["rtbit"], alpha=0.9)
        bars2 = ax.bar([i + width / 2 for i in x], qb_vals, width,
                       label="qBittorrent", color=self.COLORS["qbittorrent"], alpha=0.9)

        # Value labels on bars
        for bars in [bars1, bars2]:
            for bar in bars:
                h = bar.get_height()
                if h > 0:
                    ax.annotate(f"{h:.1f}",
                                xy=(bar.get_x() + bar.get_width() / 2, h),
                                xytext=(0, 4), textcoords="offset points",
                                ha="center", va="bottom", fontsize=8, color="#ddd")

        ax.set_ylabel("Value")
        ax.set_title(f"Metric Comparison — {scenario}", fontsize=14)
        ax.set_xticks(list(x))
        ax.set_xticklabels(labels, fontsize=9)
        ax.legend(loc="upper right")
        ax.grid(True, axis="y", alpha=0.3)
        ax.set_ylim(bottom=0)

        fig.tight_layout()
        fig.savefig(charts_dir / f"{scenario}_comparison.png", dpi=150,
                    bbox_inches="tight")
        plt.close(fig)

    def _chart_scenario_comparison(self, charts_dir: Path):
        """Compare key metrics across all scenarios."""
        scenarios = [rq.scenario for rq, _ in self.all_results]
        rq_durations = [rq.duration_sec for rq, _ in self.all_results]
        qb_durations = [qb.duration_sec for _, qb in self.all_results]
        rq_speeds = [rq.avg_speed_mbps for rq, _ in self.all_results]
        qb_speeds = [qb.avg_speed_mbps for _, qb in self.all_results]
        rq_mem = [rq.mem_peak_mb for rq, _ in self.all_results]
        qb_mem = [qb.mem_peak_mb for _, qb in self.all_results]
        rq_cpu = [rq.cpu_peak for rq, _ in self.all_results]
        qb_cpu = [qb.cpu_peak for _, qb in self.all_results]

        fig, axes = plt.subplots(2, 2, figsize=(16, 10))
        fig.suptitle("Cross-Scenario Comparison", fontsize=16, y=0.98)

        x = range(len(scenarios))
        w = 0.35

        # Duration
        ax = axes[0, 0]
        ax.bar([i - w / 2 for i in x], rq_durations, w,
               label="rtbit", color=self.COLORS["rtbit"])
        ax.bar([i + w / 2 for i in x], qb_durations, w,
               label="qBittorrent", color=self.COLORS["qbittorrent"])
        ax.set_title("Duration (seconds, lower = better)")
        ax.set_xticks(list(x))
        ax.set_xticklabels(scenarios, rotation=30, ha="right", fontsize=8)
        ax.legend(fontsize=9)
        ax.grid(True, axis="y", alpha=0.3)

        # Speed
        ax = axes[0, 1]
        ax.bar([i - w / 2 for i in x], rq_speeds, w,
               label="rtbit", color=self.COLORS["rtbit"])
        ax.bar([i + w / 2 for i in x], qb_speeds, w,
               label="qBittorrent", color=self.COLORS["qbittorrent"])
        ax.set_title("Avg Speed (Mbps, higher = better)")
        ax.set_xticks(list(x))
        ax.set_xticklabels(scenarios, rotation=30, ha="right", fontsize=8)
        ax.legend(fontsize=9)
        ax.grid(True, axis="y", alpha=0.3)

        # Memory
        ax = axes[1, 0]
        ax.bar([i - w / 2 for i in x], rq_mem, w,
               label="rtbit", color=self.COLORS["rtbit"])
        ax.bar([i + w / 2 for i in x], qb_mem, w,
               label="qBittorrent", color=self.COLORS["qbittorrent"])
        ax.set_title("Peak Memory (MB, lower = better)")
        ax.set_xticks(list(x))
        ax.set_xticklabels(scenarios, rotation=30, ha="right", fontsize=8)
        ax.legend(fontsize=9)
        ax.grid(True, axis="y", alpha=0.3)

        # CPU
        ax = axes[1, 1]
        ax.bar([i - w / 2 for i in x], rq_cpu, w,
               label="rtbit", color=self.COLORS["rtbit"])
        ax.bar([i + w / 2 for i in x], qb_cpu, w,
               label="qBittorrent", color=self.COLORS["qbittorrent"])
        ax.set_title("Peak CPU (%%, lower = better)")
        ax.set_xticks(list(x))
        ax.set_xticklabels(scenarios, rotation=30, ha="right", fontsize=8)
        ax.legend(fontsize=9)
        ax.grid(True, axis="y", alpha=0.3)

        fig.tight_layout(rect=[0, 0, 1, 0.96])
        fig.savefig(charts_dir / "cross_scenario_comparison.png", dpi=150,
                    bbox_inches="tight")
        plt.close(fig)

    def _chart_resource_efficiency(self, charts_dir: Path):
        """Scatter plot: speed vs resource usage for each scenario."""
        fig, axes = plt.subplots(1, 2, figsize=(14, 6))
        fig.suptitle("Resource Efficiency", fontsize=16, y=0.98)

        for result_set, name in [
            ([(rq,) for rq, _ in self.all_results], "rtbit"),
            ([(qb,) for _, qb in self.all_results], "qbittorrent"),
        ]:
            speeds = [r[0].avg_speed_mbps for r in result_set]
            mems = [r[0].mem_peak_mb for r in result_set]
            cpus = [r[0].cpu_peak for r in result_set]
            scenarios = [r[0].scenario for r in result_set]

            # Speed vs Memory
            ax = axes[0]
            ax.scatter(speeds, mems, label=name, color=self.COLORS[name],
                       s=100, alpha=0.8, edgecolors="white", linewidth=0.5)
            for i, sc in enumerate(scenarios):
                ax.annotate(sc, (speeds[i], mems[i]), fontsize=7,
                            xytext=(5, 5), textcoords="offset points",
                            color=self.COLORS[name], alpha=0.7)

            # Speed vs CPU
            ax = axes[1]
            ax.scatter(speeds, cpus, label=name, color=self.COLORS[name],
                       s=100, alpha=0.8, edgecolors="white", linewidth=0.5)
            for i, sc in enumerate(scenarios):
                ax.annotate(sc, (speeds[i], cpus[i]), fontsize=7,
                            xytext=(5, 5), textcoords="offset points",
                            color=self.COLORS[name], alpha=0.7)

        axes[0].set_xlabel("Avg Speed (Mbps)")
        axes[0].set_ylabel("Peak Memory (MB)")
        axes[0].set_title("Speed vs Memory")
        axes[0].legend()
        axes[0].grid(True, alpha=0.3)

        axes[1].set_xlabel("Avg Speed (Mbps)")
        axes[1].set_ylabel("Peak CPU (%)")
        axes[1].set_title("Speed vs CPU")
        axes[1].legend()
        axes[1].grid(True, alpha=0.3)

        fig.tight_layout(rect=[0, 0, 1, 0.96])
        fig.savefig(charts_dir / "resource_efficiency.png", dpi=150,
                    bbox_inches="tight")
        plt.close(fig)

    def _chart_dashboard(self, charts_dir: Path):
        """Single-page overview dashboard with all key findings."""
        n_scenarios = len(self.all_results)
        fig = plt.figure(figsize=(18, 6 + 4 * n_scenarios))
        gs = GridSpec(1 + n_scenarios, 3, figure=fig, hspace=0.4, wspace=0.3)

        fig.suptitle("Benchmark Dashboard: rtbit vs qBittorrent",
                     fontsize=18, y=0.99, fontweight="bold")

        # ── Row 0: Summary bar charts ────────────────────────────────────
        # Speedup ratio
        ax0 = fig.add_subplot(gs[0, 0])
        scenarios = [rq.scenario for rq, _ in self.all_results]
        speedups = []
        for rq, qb in self.all_results:
            if rq.duration_sec > 0 and qb.duration_sec > 0:
                speedups.append(qb.duration_sec / rq.duration_sec)
            else:
                speedups.append(1.0)
        colors = ["#4CAF50" if s >= 1.0 else "#FF5722" for s in speedups]
        bars = ax0.barh(scenarios, speedups, color=colors, alpha=0.85)
        ax0.axvline(x=1.0, color="#888", linestyle="--", linewidth=1)
        ax0.set_xlabel("Speed Ratio (>1 = rtbit faster)")
        ax0.set_title("Relative Performance")
        for bar, val in zip(bars, speedups):
            ax0.text(bar.get_width() + 0.02, bar.get_y() + bar.get_height() / 2,
                     f"{val:.2f}x", va="center", fontsize=9, color="#ddd")
        ax0.set_xlim(left=0)

        # Memory comparison
        ax1 = fig.add_subplot(gs[0, 1])
        rq_mem = [rq.mem_peak_mb for rq, _ in self.all_results]
        qb_mem = [qb.mem_peak_mb for _, qb in self.all_results]
        x = range(len(scenarios))
        w = 0.35
        ax1.bar([i - w / 2 for i in x], rq_mem, w,
                label="rtbit", color=self.COLORS["rtbit"])
        ax1.bar([i + w / 2 for i in x], qb_mem, w,
                label="qBittorrent", color=self.COLORS["qbittorrent"])
        ax1.set_title("Peak Memory (MB)")
        ax1.set_xticks(list(x))
        ax1.set_xticklabels(scenarios, rotation=30, ha="right", fontsize=8)
        ax1.legend(fontsize=8)
        ax1.grid(True, axis="y", alpha=0.3)

        # CPU comparison
        ax2 = fig.add_subplot(gs[0, 2])
        rq_cpu = [rq.cpu_peak for rq, _ in self.all_results]
        qb_cpu = [qb.cpu_peak for _, qb in self.all_results]
        ax2.bar([i - w / 2 for i in x], rq_cpu, w,
                label="rtbit", color=self.COLORS["rtbit"])
        ax2.bar([i + w / 2 for i in x], qb_cpu, w,
                label="qBittorrent", color=self.COLORS["qbittorrent"])
        ax2.set_title("Peak CPU (%)")
        ax2.set_xticks(list(x))
        ax2.set_xticklabels(scenarios, rotation=30, ha="right", fontsize=8)
        ax2.legend(fontsize=8)
        ax2.grid(True, axis="y", alpha=0.3)

        # ── Per-scenario rows: timeseries mini-panels ────────────────────
        for row_idx, (rq, qb) in enumerate(self.all_results, start=1):
            scenario = rq.scenario

            # CPU time series
            ax_cpu = fig.add_subplot(gs[row_idx, 0])
            for result, name in [(rq, "rtbit"), (qb, "qbittorrent")]:
                if result.timeseries:
                    t = self._ts_to_relative(result.timeseries)
                    vals = [p.get("cpu_pct", 0) for p in result.timeseries]
                    ax_cpu.plot(t, vals, label=name, color=self.COLORS[name], lw=1.5)
                    ax_cpu.fill_between(t, vals, alpha=0.1, color=self.COLORS[name])
            ax_cpu.set_title(f"{scenario} — CPU %", fontsize=10)
            ax_cpu.set_ylabel("%")
            ax_cpu.legend(fontsize=7, loc="upper right")
            ax_cpu.grid(True, alpha=0.3)
            ax_cpu.set_xlim(left=0)
            ax_cpu.set_ylim(bottom=0)

            # Memory time series
            ax_mem = fig.add_subplot(gs[row_idx, 1])
            for result, name in [(rq, "rtbit"), (qb, "qbittorrent")]:
                if result.timeseries:
                    t = self._ts_to_relative(result.timeseries)
                    vals = [p.get("mem_bytes", 0) / MB for p in result.timeseries]
                    ax_mem.plot(t, vals, label=name, color=self.COLORS[name], lw=1.5)
                    ax_mem.fill_between(t, vals, alpha=0.1, color=self.COLORS[name])
            ax_mem.set_title(f"{scenario} — Memory MB", fontsize=10)
            ax_mem.set_ylabel("MB")
            ax_mem.legend(fontsize=7, loc="upper right")
            ax_mem.grid(True, alpha=0.3)
            ax_mem.set_xlim(left=0)
            ax_mem.set_ylim(bottom=0)

            # IO Wait time series
            ax_io = fig.add_subplot(gs[row_idx, 2])
            for result, name in [(rq, "rtbit"), (qb, "qbittorrent")]:
                if result.timeseries:
                    t = self._ts_to_relative(result.timeseries)
                    vals = [p.get("iowait_pct", 0) for p in result.timeseries]
                    ax_io.plot(t, vals, label=name, color=self.COLORS[name], lw=1.5)
                    ax_io.fill_between(t, vals, alpha=0.1, color=self.COLORS[name])
            ax_io.set_title(f"{scenario} — IO Wait %", fontsize=10)
            ax_io.set_ylabel("%")
            ax_io.legend(fontsize=7, loc="upper right")
            ax_io.grid(True, alpha=0.3)
            ax_io.set_xlim(left=0)
            ax_io.set_ylim(bottom=0)

            # X label on bottom row only
            if row_idx == n_scenarios:
                ax_cpu.set_xlabel("Time (s)")
                ax_mem.set_xlabel("Time (s)")
                ax_io.set_xlabel("Time (s)")

        fig.savefig(charts_dir / "dashboard.png", dpi=150, bbox_inches="tight")
        plt.close(fig)

    def _write_csv(self, path: Path):
        fields = [
            "scenario", "client", "total_bytes", "duration_sec",
            "avg_speed_mbps", "peak_speed_mbps", "time_to_first_piece",
            "cpu_avg", "cpu_peak", "mem_avg_mb", "mem_peak_mb",
            "net_rx_avg_mbps", "net_rx_peak_mbps",
            "disk_write_avg_mbps", "disk_write_peak_mbps",
            "iowait_avg", "iowait_peak",
        ]
        with open(path, "w") as f:
            f.write(",".join(fields) + "\n")
            for rq, qb in self.all_results:
                for r in [rq, qb]:
                    vals = [str(getattr(r, field, "")) for field in fields]
                    f.write(",".join(vals) + "\n")


# ═══════════════════════════════════════════════════════════════════════════════
# Main
# ═══════════════════════════════════════════════════════════════════════════════

def main():
    try:
        runner = BenchmarkRunner()
        runner.run()
    except KeyboardInterrupt:
        log.info("Interrupted.")
        sys.exit(1)
    except Exception as e:
        log.error("Fatal: %s", e, exc_info=True)
        sys.exit(1)


if __name__ == "__main__":
    main()
