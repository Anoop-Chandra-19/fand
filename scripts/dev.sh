#!/usr/bin/env bash
# Launch the GUI against a throwaway development daemon — never the live
# service, never real hardware writes, never the real config file.
#
#   scripts/dev.sh real    repo-built fand in --dry-run (real sensors, no writes)
#   scripts/dev.sh mock    mockd with synthetic data
#                          (SCENARIO=normal|heat-ramp|flappy|restart)
#
# Everything (socket + a copy of the example config) lives in a temp dir
# that is removed on exit; the daemon is killed when the GUI closes.
set -euo pipefail

mode=${1:?usage: scripts/dev.sh real|mock}
root=$(cd -- "$(dirname -- "$0")/.." && pwd)
dir=$(mktemp -d -t fand-dev.XXXXXX)
sock=$dir/fand.sock
cp "$root/config/fand.example.toml" "$dir/config.toml"

daemon_pid=
cleanup() {
    if [ -n "$daemon_pid" ] && kill -0 "$daemon_pid" 2>/dev/null; then
        kill "$daemon_pid" 2>/dev/null || true
        wait "$daemon_pid" 2>/dev/null || true
    fi
    rm -rf "$dir"
}
trap cleanup EXIT

case $mode in
real)
    cargo build -p fand
    "$root/target/debug/fand" --dry-run --config "$dir/config.toml" --socket "$sock" &
    ;;
mock)
    cargo build -p fand --example mockd
    "$root/target/debug/examples/mockd" --config "$dir/config.toml" --socket "$sock" \
        --scenario "${SCENARIO:-normal}" &
    ;;
*)
    echo "unknown mode: $mode (expected real|mock)" >&2
    exit 2
    ;;
esac
daemon_pid=$!

# Wait for the daemon to bind its socket before pointing the GUI at it.
for _ in $(seq 100); do
    [ -S "$sock" ] && break
    if ! kill -0 "$daemon_pid" 2>/dev/null; then
        echo "daemon exited before creating its socket" >&2
        exit 1
    fi
    sleep 0.1
done

# No exec: the EXIT trap must still run to kill the daemon and clean up.
cd "$root/gui"
FAND_SOCKET=$sock npm run tauri dev
