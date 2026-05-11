#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

cat <<'BANNER'
╔═══════════════════════════════════════════════════════╗
║  BitTorrent Client Benchmark: rtbit vs qBittorrent   ║
╚═══════════════════════════════════════════════════════╝
BANNER

# ── Parse args ───────────────────────────────────────────────────────────────
usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --scenarios GROUP   Scenario group or comma-separated names (default: quick)"
    echo "  --seeders N         Max real seeder instances (default: 10)"
    echo "  --mock-peers N      Mock peer count per mock-seeder container (default: 100)"
    echo "  --quick             Skip cleanup, reuse existing containers"
    echo "  --no-cleanup        Keep containers running after benchmark"
    echo "  --help              Show this help"
    echo ""
    echo "Scenario groups:"
    echo "  quick       - 2 GB x 1 file x 3 peers (smoke test, ~5 min)"
    echo "  medium      - 6 scenarios covering all axes (~15-20 min)"
    echo "  size_ramp   - 2-20 GB x 1 file x 3 peers (10 scenarios, ~2 hrs)"
    echo "  file_ramp   - 10 GB x {1,10,50,100} files x 3 peers (4 scenarios, ~1 hr)"
    echo "  peer_ramp   - 10 GB x 1 file x {3..1000} peers (7 scenarios, ~2 hrs)"
    echo "  all         - Full matrix: 10 sizes x 4 file counts x 7 peer configs (~days)"
    echo ""
    echo "Individual scenario names follow the pattern: sz{N}g_f{M}_{P}p"
    echo "  sz10g_f1_100p  = 10 GB total, 1 file, 100 mock peers"
    echo "  sz20g_f50_3p   = 20 GB total, 50 files, 3 real seeders"
    echo ""
    echo "Examples:"
    echo "  $0 --scenarios quick                         # Smoke test"
    echo "  $0 --scenarios size_ramp                     # Size ramp only"
    echo "  $0 --scenarios sz10g_f1_3p,sz10g_f1_100p     # Specific scenarios"
    echo "  $0 --scenarios peer_ramp --mock-peers 1000   # Peer scaling"
    echo "  $0 --scenarios all                           # Everything (~hours)"
    exit 0
}

SCENARIOS="${SCENARIOS:-quick}"
MAX_SEEDERS="${MAX_SEEDERS:-10}"
MOCK_PEERS="${MOCK_PEERS:-100}"
QUICK=0
CLEANUP=1

while [[ $# -gt 0 ]]; do
    case "$1" in
        --scenarios)   SCENARIOS="$2"; shift 2 ;;
        --seeders)     MAX_SEEDERS="$2"; shift 2 ;;
        --mock-peers)  MOCK_PEERS="$2"; shift 2 ;;
        --quick)       QUICK=1; shift ;;
        --no-cleanup)  CLEANUP=0; shift ;;
        --help|-h)     usage ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

export SCENARIOS MAX_SEEDERS MOCK_PEERS QUICK

# ── Prepare ──────────────────────────────────────────────────────────────────
mkdir -p results

if [[ "$QUICK" != "1" ]]; then
    echo "[*] Cleaning up previous run..."
    docker compose down -v 2>/dev/null || true
fi

# ── Build & run ──────────────────────────────────────────────────────────────
echo "[*] Scenarios:   $SCENARIOS"
echo "[*] Seeders:     $MAX_SEEDERS real, $MOCK_PEERS mock"
echo "[*] Building and starting services..."
echo "    (First run builds rtbit from source — this takes a while)"
echo ""

docker compose up --build --abort-on-container-exit --exit-code-from orchestrator
EXIT_CODE=$?

# ── Cleanup ──────────────────────────────────────────────────────────────────
if [[ "$CLEANUP" == "1" ]]; then
    echo ""
    echo "[*] Cleaning up Docker resources..."
    docker compose down -v 2>/dev/null || true
fi

echo ""
echo "════════════════════════════════════════════════════════"
echo "  Results saved to: $(pwd)/results/"
ls -lh results/ 2>/dev/null | tail -5
echo "════════════════════════════════════════════════════════"

exit "${EXIT_CODE}"
