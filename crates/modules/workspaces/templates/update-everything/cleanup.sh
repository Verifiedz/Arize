# Cleanup (supervised): close the system update's terminal if it's still open, and clear the
# last run. Run by `workspaces stop`, and by `workspaces cleanup` after a run that went dirty
# (e.g. the terminal never opened).
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-update.sh"

shimmer_close_windows
rm -f "$SHIMMER_RESULTS" "$SHIMMER_STATE_DIR/system.exit" "$SHIMMER_STATE_DIR/system.pid"
echo "nothing else to undo: updates stay installed"
