# Cleanup (supervised): stop the background step, if it's still running.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

file="$SHIMMER_STATE_DIR/background.pid"
if [ -f "$file" ]; then
    pid=$(cat "$file")
    if kill -0 "$pid" 2>/dev/null; then
        shimmer_stop_group "$pid"
        echo "stopped the background step ($pid)"
    fi
    rm -f "$file"
fi

[ "$FAIL_AT" = "cleanup" ] && shimmer_fail "FAIL_AT = cleanup: failing on purpose"
echo "cleaned up"
