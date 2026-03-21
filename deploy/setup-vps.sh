#!/bin/bash
# One-time VPS setup script for rustTorrent website
# Run on VPS (46.250.255.234) as root
#
# Prerequisites:
#   1. rusttorrent.dev purchased and DNS pointing to VPS (46.250.255.234)
#   2. rusttorrent.dev zone added to existing CLOUDFLARE_API_TOKEN in Cloudflare
#   3. GitHub repo AusAgentSmith/rustTorrent_Website exists and pushed
#   4. Gitea mirror of the repo created

set -e

echo "=== rustTorrent Website VPS Setup ==="

# 1. Clone the repo
echo "[1/5] Cloning repository..."
if [ -d ~/rustTorrent_Website ]; then
    echo "  Repository already exists, pulling latest..."
    cd ~/rustTorrent_Website && git pull origin main
else
    cd ~ && git clone https://github.com/AusAgentSmith/rustTorrent_Website.git
fi

# 2. Copy deploy script
echo "[2/5] Installing deploy script..."
cp ~/rustTorrent_Website/deploy/deploy-rusttorrent.sh ~/Caddy/webhook/scripts/deploy-rusttorrent.sh
chmod +x ~/Caddy/webhook/scripts/deploy-rusttorrent.sh

# 3. Merge webhook hook into hooks.json
echo "[3/5] Webhook hook config..."
echo "  MANUAL STEP: Merge deploy/webhook-hook.json into ~/Caddy/webhook/hooks.json"
echo "  Add the deploy-rusttorrent-website entry to the existing JSON array"

# 4. Update Caddy config
echo "[4/5] Caddy config..."
echo "  MANUAL STEP: Add the rusttorrent.dev block from deploy/vps-caddyfile-block.txt to ~/Caddy/Caddyfile"
echo "  MANUAL STEP: Add volume mount and env var to ~/Caddy/docker-compose.yml (see deploy/docker-compose-caddy-additions.yml)"
echo "  NOTE: rusttorrent.dev uses existing CLOUDFLARE_API_TOKEN — no new env var needed"

# 5. Restart services
echo "[5/5] After manual steps, restart Caddy and webhook:"
echo "  cd ~/Caddy && docker compose up -d"
echo ""
echo "=== Setup complete (manual steps required above) ==="
echo ""
echo "Gitea mirror setup:"
echo "  1. Go to gitea.ausagentsmith.com → New Migration"
echo "  2. Source: https://github.com/AusAgentSmith/rustTorrent_Website.git"
echo "  3. Name: rustTorrent_Website, Owner: AusAgentSmith"
echo "  4. Mirror interval: 10 minutes"
echo "  5. Add push webhook:"
echo "     URL: https://deploy.indexarr.net/hooks/deploy-rusttorrent-website"
echo "     Secret: d951290d1720a988ec66711cc74c8b0e60b2a416359548098ff2fec03c294adc"
echo "     Content type: application/json"
echo "     Events: Push"
