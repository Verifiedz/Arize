# Cleanup (supervised): stop the dev server this workspace started, freeing its port.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

pid=$(shimmer_dev_server_pid)
if [ -n "$pid" ]; then
    shimmer_stop_group "$pid"
    echo "stopped the dev server ($pid)"
else
    echo "no dev server running"
fi
rm -f "$SHIMMER_STATE_DIR/dev-server.pid"
