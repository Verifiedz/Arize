# Cleanup (supervised): close the system clean's terminal if it's still open. Nothing removed
# comes back from here: a rebuild, reinstall or download does that.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-free.sh"

shimmer_close_windows
rm -f "$SHIMMER_RESULTS" "$SHIMMER_STATE_DIR/avail-before" "$SHIMMER_STATE_DIR/system.exit" "$SHIMMER_STATE_DIR/system.pid"
echo "nothing to undo: build folders and caches come back when a tool needs them"
