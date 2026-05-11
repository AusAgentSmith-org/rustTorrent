#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

cat <<'BANNER'
╔═══════════════════════════════════════════════════════════╗
║  benchv2: rtbit vs qBittorrent (Rust-native)             ║
╚═══════════════════════════════════════════════════════════╝
BANNER

usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --scenarios GROUP   quick|medium|size_ramp|file_ramp|peer_ramp|all"
    echo "  --seeders N         Real seeder instances (default: 10)"
    echo "  --mock-peers N      Mock peers per container (default: 100)"
    echo "  --no-cleanup        Keep containers after run"
    echo "  --help              Show this help"
    echo ""
    echo "Scenario groups:"
    echo "  quick    - 8 GB x 1 file x 3 peers (~5 min)"
    echo "  medium   - 6 scenarios 8-16 GB, all axes (~30-45 min)"
    echo "  all      - Full 280-scenario matrix"
    exit 0
}

SCENARIOS="${SCENARIOS:-quick}"
MAX_SEEDERS="${MAX_SEEDERS:-10}"
MOCK_PEERS="${MOCK_PEERS:-100}"
CLEANUP=1

while [[ $# -gt 0 ]]; do
    case "$1" in
        --scenarios)   SCENARIOS="$2"; shift 2 ;;
        --seeders)     MAX_SEEDERS="$2"; shift 2 ;;
        --mock-peers)  MOCK_PEERS="$2"; shift 2 ;;
        --no-cleanup)  CLEANUP=0; shift ;;
        --help|-h)     usage ;;
        *) echo "Unknown: $1"; usage ;;
    esac
done

export SCENARIOS MAX_SEEDERS MOCK_PEERS

mkdir -p results
LOGFILE="results/run_$(date +%Y%m%d_%H%M%S).log"

docker compose down -v 2>/dev/null || true

echo "[*] Scenarios:   $SCENARIOS"
echo "[*] Seeders:     $MAX_SEEDERS real, $MOCK_PEERS mock"
echo "[*] Log file:    $LOGFILE"
echo "[*] Building (first run compiles Rust — takes a few minutes)..."
echo ""

# Run and tee output to both console and log file
docker compose up --build --abort-on-container-exit --exit-code-from orchestrator 2>&1 | tee "$LOGFILE"
EXIT_CODE=${PIPESTATUS[0]}

[[ "$CLEANUP" == "1" ]] && docker compose down -v 2>/dev/null || true

echo ""
echo "════════════════════════════════════════════"
echo "  Results: $(pwd)/results/"
echo "  Log:     $LOGFILE"
ls -lh results/ 2>/dev/null | tail -5
echo "════════════════════════════════════════════"
exit "${EXIT_CODE}"
