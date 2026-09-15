#!/usr/bin/env bash
# Run from the app directory: ./scripts/sync-repology.sh /path/to/data [FORCE_IMPORT]
# Set FORCE_IMPORT to 1 to import even when the dump hasn't changed and download is skipped.
set -euo pipefail

dir=$(realpath "${1:?usage: $0 DATA_DIR [FORCE_IMPORT]}")
force_import=${2:-0}
dump="$dir/repology-database-dump-latest.sql.zst"
pending="$dump.pending"
tmp=/tmp/data.db
stopped=false
backed_up=false

[[ -f "$dir/data.db" ]] || { echo "missing $dir/data.db" >&2; exit 1; }
[[ "$dir" != /tmp ]] || { echo "DATA_DIR must not be /tmp" >&2; exit 1; }

# Global lock check.
exec 9>/tmp/pkgbot-sync-repology.lock
flock -n 9 || { echo "repology sync is already running"; exit 0; }

cleanup() {
    status=$?
    trap - EXIT
    if $stopped; then
        if $backed_up; then
            mv -f "$dir/data.db.bak" "$dir/data.db" || true
        fi
        sudo -n systemctl start pkgbot || true
    fi
    rm -f "$dump.part" "$dir/data.db.new"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Download the file and retain timestamps for cron freshness checks.
# Skip an unchanged dump unless an import is forced or needs retrying.
condition=()
[[ ! -f "$dump" ]] || condition=(--time-cond "$dump")
code=$(curl --fail --location --show-error --silent --remote-time \
    "${condition[@]}" --output "$dump.part" --write-out '%{http_code}' \
    https://dumps.repology.org/repology-database-dump-latest.sql.zst)

case "$code" in
    200)
        touch "$pending"
        mv -f "$dump.part" "$dump"
        ;;
    304)
        if [[ "$force_import" == 1 ]]; then
            echo "dump unchanged. forcing import ..."
            touch "$pending"
        elif [[ -f "$pending" ]]; then
            echo "retrying the previous unfinished import ..."
        else
            echo "no new repology dump. exiting ..."
            exit 0
        fi
        ;;
    *) echo "unexpected HTTP status: $code" >&2; exit 1 ;;
esac

# Remove the old PG volume completely.
docker compose down db -v

# Wait for PG to be up and run the import.
docker compose up -d --wait --wait-timeout 120 db
# Stream only the tables the importer needs, without writing an extracted SQL file.
zstd --decompress --stdout "$dump" | ./pkgbot restore-repology

# Run the import.
rm -f "$tmp" "$tmp-wal" "$tmp-shm"
./pkgbot --db=/tmp/data.db install --yes
./pkgbot --db=/tmp/data.db import

# Stop docker once the import is done.
docker compose down db

# Step the new DB's attribs.
mv -f "$tmp" "$dir/data.db.new"
chmod --reference="$dir/data.db" "$dir/data.db.new"
sudo -n chown --reference="$dir/data.db" "$dir/data.db.new"

# Create a backup, stop the app, and replace the old DB with the new one.
stopped=true
sudo -n systemctl stop pkgbot
mv -f "$dir/data.db" "$dir/data.db.bak"
backed_up=true
mv -f "$dir/data.db.new" "$dir/data.db"

# Start!
sudo -n systemctl start pkgbot
stopped=false
rm -f "$pending"
echo "repology sync complete"
