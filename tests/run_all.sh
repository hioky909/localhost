#!/usr/bin/env bash
# Run the localhost server in the background, execute the test suite,
# then kill the server.
set -e

PROJ="/mnt/c/Users/axelw/localhost"
BINARY="$PROJ/target/debug/localhost"

echo "=== Building server ==="
cd "$PROJ"
~/.cargo/bin/cargo build --quiet

echo ""
echo "=== Starting server ==="
"$BINARY" &
SERVER_PID=$!
echo "Server PID: $SERVER_PID"

# Wait for server to bind
sleep 2

echo ""
echo "=== Running tests ==="
python3 "$PROJ/tests/run_tests.py"
TEST_EXIT=$?

echo ""
echo "=== Stopping server (PID $SERVER_PID) ==="
kill "$SERVER_PID" 2>/dev/null || true
wait "$SERVER_PID" 2>/dev/null || true

exit $TEST_EXIT
