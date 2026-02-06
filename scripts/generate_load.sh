#!/usr/bin/env bash
set -euo pipefail

PORT=27017
RS=rs0
DBPATH=$(mktemp -d /tmp/mongo-ftdc-XXXXXX)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DATA_DIR="$SCRIPT_DIR/../test-data"
WORKLOAD_SCRIPT="$SCRIPT_DIR/workload.js"

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 /path/to/mongo/bin"
  exit 1
fi

MONGO_BIN="$1"

MONGOD="$MONGO_BIN/mongod"
MONGOSH="$MONGO_BIN/mongosh"

if [[ ! -x "$MONGOD" ]]; then
  echo "ERROR: mongod not executable: $MONGOD"
  exit 1
fi

if [[ ! -x "$MONGOSH" ]]; then
  echo "ERROR: mongosh not executable: $MONGOSH"
  exit 1
fi

echo "Using:"
echo "  mongod : $MONGOD"
echo "  mongosh: $MONGOSH"
echo "dbpath: $DBPATH"

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

echo "mongod pid=$MONGOD_PID"

echo "Waiting for mongod..."
until "$MONGOSH" --quiet --eval "db.runCommand({ ping: 1 })" >/dev/null 2>&1; do
  sleep 0.2
done

echo "Initiating replica set..."
"$MONGOSH" --quiet --eval "
rs.initiate({
  _id: \"$RS\",
  members: [{ _id: 0, host: \"127.0.0.1:$PORT\" }]
})
"

echo "Waiting for node to become PRIMARY..."
until "$MONGOSH" --quiet --eval "rs.hello().isWritablePrimary" | grep true >/dev/null; do
  sleep 0.2
done

echo "Running workload..."
"$MONGOSH" \
  --quiet \
  "mongodb://127.0.0.1:$PORT/db?replicaSet=$RS" \
  --eval "globalThis.__args = { coll:\"test\", workers:8, seconds:60, docSize:512, useTxn:false };" \
  "$WORKLOAD_SCRIPT"

echo "Workload complete."

FTDC_SRC="$DBPATH/diagnostic.data"
DEST="$TEST_DATA_DIR/diagnostic.data"

if [[ ! -d "$FTDC_SRC" ]]; then
  echo "ERROR: diagnostic.data not found!"
  exit 1
fi

echo "Copying FTDC data to $DEST"
rm -rf "$DEST"
cp -a "$FTDC_SRC" "$DEST"

echo "FTDC copied to: $DEST"
