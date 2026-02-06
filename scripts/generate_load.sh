#!/usr/bin/env bash
set -euo pipefail

# Defaults
PORT=27017
DURATION=60
WORKERS=8
DOC_SIZE=512

usage() {
  cat <<EOF
Usage: $(basename "$0") [options] /path/to/mongo/bin

Starts a temporary mongod, runs a CRUD workload, and copies the
resulting FTDC data into test-data/diagnostic.data.

Options:
  -d, --duration <secs>   Workload duration (default: $DURATION)
  -w, --workers <n>       Concurrent workers (default: $WORKERS)
  -s, --doc-size <bytes>  Approx document payload size (default: $DOC_SIZE)
  -p, --port <port>       mongod port (default: $PORT)
  -h, --help              Show this help

Examples:
  $(basename "$0") ./mongodb/bin
  $(basename "$0") -d 120 -w 4 ./mongodb/bin
  $(basename "$0") --duration 30 --port 27018 ./mongodb/bin
EOF
  exit "${1:-0}"
}

# Parse args
MONGO_BIN=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -d|--duration) DURATION="$2"; shift 2 ;;
    -w|--workers)  WORKERS="$2"; shift 2 ;;
    -s|--doc-size) DOC_SIZE="$2"; shift 2 ;;
    -p|--port)     PORT="$2"; shift 2 ;;
    -h|--help)     usage 0 ;;
    -*)            echo "Unknown option: $1"; usage 1 ;;
    *)             MONGO_BIN="$1"; shift ;;
  esac
done

if [[ -z "$MONGO_BIN" ]]; then
  echo "Error: mongo bin path is required"
  usage 1
fi

RS=rs0
DBPATH=$(mktemp -d /tmp/mongo-ftdc-XXXXXX)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DATA_DIR="$SCRIPT_DIR/../test-data"
WORKLOAD_SCRIPT="$SCRIPT_DIR/workload.js"

MONGOD="$MONGO_BIN/mongod"
MONGOSH="$MONGO_BIN/mongosh"

if [[ ! -x "$MONGOD" ]]; then
  echo "Error: mongod not found at $MONGOD"
  exit 1
fi

if [[ ! -x "$MONGOSH" ]]; then
  echo "Error: mongosh not found at $MONGOSH"
  exit 1
fi

echo "Config:"
echo "  mongod:   $MONGOD"
echo "  port:     $PORT"
echo "  duration: ${DURATION}s"
echo "  workers:  $WORKERS"
echo "  docSize:  ${DOC_SIZE}B"
echo "  dbpath:   $DBPATH"
echo ""

cleanup() {
  echo "Shutting down mongod..."
  if kill -0 "$MONGOD_PID" 2>/dev/null; then
    kill "$MONGOD_PID"
    wait "$MONGOD_PID" || true
  fi
}
trap cleanup EXIT

mkdir -p "$TEST_DATA_DIR"

echo "Starting mongod..."
"$MONGOD" \
  --port "$PORT" \
  --dbpath "$DBPATH" \
  --replSet "$RS" \
  --bind_ip 127.0.0.1 \
  --quiet \
  > "$DBPATH/mongod.log" 2>&1 &

MONGOD_PID=$!

echo "Waiting for mongod (pid=$MONGOD_PID)..."
until "$MONGOSH" --port "$PORT" --quiet --eval "db.runCommand({ ping: 1 })" >/dev/null 2>&1; do
  sleep 0.2
done

echo "Initiating replica set..."
"$MONGOSH" --port "$PORT" --quiet --eval "
rs.initiate({
  _id: \"$RS\",
  members: [{ _id: 0, host: \"127.0.0.1:$PORT\" }]
})
"

echo "Waiting for PRIMARY..."
until "$MONGOSH" --port "$PORT" --quiet --eval "rs.hello().isWritablePrimary" | grep true >/dev/null; do
  sleep 0.2
done

echo "Running workload..."
"$MONGOSH" \
  --quiet \
  "mongodb://127.0.0.1:$PORT/db?replicaSet=$RS" \
  --eval "globalThis.__args = { coll:\"test\", workers:$WORKERS, seconds:$DURATION, docSize:$DOC_SIZE, useTxn:false };" \
  "$WORKLOAD_SCRIPT"

echo "Workload complete."

FTDC_SRC="$DBPATH/diagnostic.data"
DEST="$TEST_DATA_DIR/diagnostic.data"

if [[ ! -d "$FTDC_SRC" ]]; then
  echo "Error: diagnostic.data not found at $FTDC_SRC"
  exit 1
fi

echo "Copying FTDC data to $DEST"
rm -rf "$DEST"
cp -a "$FTDC_SRC" "$DEST"

echo "Done! FTDC data at: $DEST"
