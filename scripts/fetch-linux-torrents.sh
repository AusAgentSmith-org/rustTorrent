#!/usr/bin/env bash
#
# Fetch .torrent files for popular Linux ISOs from official sources.
# Usage: ./fetch-linux-torrents.sh [output_dir]
#
set -euo pipefail

OUTDIR="${1:-./linux-torrents}"
mkdir -p "$OUTDIR"

TOTAL=0
FAILED=0

download() {
    local url="$1"
    local name
    name=$(basename "$url" | sed 's/[?].*//')
    if [[ -f "$OUTDIR/$name" ]]; then
        echo "  SKIP (exists): $name"
        return
    fi
    if curl -fsSL --max-time 30 -o "$OUTDIR/$name" "$url"; then
        echo "  OK: $name"
        TOTAL=$((TOTAL + 1))
    else
        echo "  FAIL: $name"
        rm -f "$OUTDIR/$name"
        FAILED=$((FAILED + 1))
    fi
}

# Scrape a page for .torrent links and download them
scrape_and_download() {
    local page_url="$1"
    local base_url="$2"
    local label="$3"
    local filter="${4:-\.torrent}"

    echo "=== $label ==="
    echo "  Scraping: $page_url"

    local links
    links=$(curl -fsSL --max-time 30 "$page_url" 2>/dev/null \
        | grep -oiE 'href="[^"]*'"$filter"'[^"]*"' \
        | sed 's/href="//i;s/"$//' \
        | sort -u) || true

    if [[ -z "$links" ]]; then
        echo "  No .torrent links found"
        return
    fi

    while IFS= read -r link; do
        # Make relative URLs absolute
        if [[ "$link" =~ ^https?:// ]]; then
            download "$link"
        elif [[ "$link" =~ ^/ ]]; then
            download "${base_url}${link}"
        else
            download "${page_url%/}/${link}"
        fi
    done <<< "$links"
}

echo "Downloading Linux ISO .torrent files to: $OUTDIR"
echo ""

# --- Ubuntu ---
for release in noble jammy; do
    scrape_and_download \
        "https://releases.ubuntu.com/$release/" \
        "https://releases.ubuntu.com" \
        "Ubuntu ($release)" \
        '\.torrent'
done

# --- Debian ---
scrape_and_download \
    "https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/" \
    "https://cdimage.debian.org" \
    "Debian (amd64 CD)" \
    '\.torrent'

scrape_and_download \
    "https://cdimage.debian.org/debian-cd/current/amd64/bt-dvd/" \
    "https://cdimage.debian.org" \
    "Debian (amd64 DVD)" \
    '\.torrent'

# --- Fedora ---
# Fedora provides direct torrent links
for variant in Workstation Server; do
    for ver in 41 40; do
        url="https://download.fedoraproject.org/pub/fedora/linux/releases/${ver}/${variant}/x86_64/iso/"
        scrape_and_download \
            "$url" \
            "https://download.fedoraproject.org" \
            "Fedora $ver $variant" \
            '\.torrent'
    done
done

# --- Arch Linux ---
scrape_and_download \
    "https://archlinux.org/releng/releases/" \
    "https://archlinux.org" \
    "Arch Linux" \
    '\.torrent'

# Also try the direct latest torrent
echo "=== Arch Linux (latest) ==="
download "https://archlinux.org/releng/releases/latest/torrent/"

# --- Linux Mint ---
for edition in cinnamon mate xfce; do
    scrape_and_download \
        "https://www.linuxmint.com/torrents/" \
        "https://www.linuxmint.com" \
        "Linux Mint ($edition)" \
        "${edition}.*\.torrent"
done

# --- openSUSE ---
scrape_and_download \
    "https://get.opensuse.org/tumbleweed/" \
    "https://get.opensuse.org" \
    "openSUSE Tumbleweed" \
    '\.torrent'

# --- Manjaro ---
for edition in gnome kde xfce; do
    scrape_and_download \
        "https://manjaro.org/products/download/$edition" \
        "https://manjaro.org" \
        "Manjaro ($edition)" \
        '\.torrent'
done

# --- Rocky Linux ---
scrape_and_download \
    "https://download.rockylinux.org/pub/rocky/9/isos/x86_64/" \
    "https://download.rockylinux.org" \
    "Rocky Linux 9" \
    '\.torrent'

# --- AlmaLinux ---
scrape_and_download \
    "https://repo.almalinux.org/almalinux/9/isos/x86_64/" \
    "https://repo.almalinux.org" \
    "AlmaLinux 9" \
    '\.torrent'

# --- Kali Linux ---
scrape_and_download \
    "https://cdimage.kali.org/current/" \
    "https://cdimage.kali.org" \
    "Kali Linux" \
    '\.torrent'

# --- MX Linux ---
scrape_and_download \
    "https://mxlinux.org/wiki/system/iso-download-mirrors/" \
    "https://mxlinux.org" \
    "MX Linux" \
    '\.torrent'

# --- Magnet links from Linuxtracker (popular meta-site) ---
echo ""
echo "=== Linuxtracker (popular distros) ==="
# Linuxtracker lists many distros; scrape the front page for .torrent links
scrape_and_download \
    "https://linuxtracker.org/index.php?page=torrents&active=1&category=0&order=5&by=2" \
    "https://linuxtracker.org" \
    "Linuxtracker (most seeded)" \
    '\.torrent'

echo ""
echo "========================================"
echo "Done! Downloaded $TOTAL .torrent files to $OUTDIR"
[[ $FAILED -gt 0 ]] && echo "($FAILED downloads failed)"
echo ""

# Show what we got
if command -v ls &>/dev/null && [[ $TOTAL -gt 0 ]]; then
    echo "Files:"
    ls -1 "$OUTDIR"/*.torrent 2>/dev/null | head -50
    count=$(ls -1 "$OUTDIR"/*.torrent 2>/dev/null | wc -l)
    [[ $count -gt 50 ]] && echo "  ... and $((count - 50)) more"
fi
