# Cleanup (supervised): close the terminal it opened when it can (CLOSE_ON_STOP), and list what
# it left open. The scratch folder is always kept.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-scratch.sh"

shimmer_close_windows
dir=$(shimmer_scratch_current)
[ -n "$dir" ] && echo "kept: $dir"
rm -f "$SHIMMER_STATE_DIR/current"
exit 0
