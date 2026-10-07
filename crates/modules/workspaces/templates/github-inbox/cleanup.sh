# Cleanup (supervised): nothing was changed on GitHub or here, so nothing to undo. Browser tabs
# it opened stay open (they're in your own browser window). Clears the last inbox.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-github.sh"

rm -f "$SHIMMER_RESULTS" "$SHIMMER_STATE_DIR/unread"
[ "${OPEN:-nothing}" = "nothing" ] || echo "left open, close them yourself: the GitHub tabs (they're in your own browser)"
echo "nothing to undo: the inbox only reads"
