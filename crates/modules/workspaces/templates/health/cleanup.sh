# Cleanup (supervised): the health check changes nothing and opens nothing, so there's nothing
# to undo. Clears the last report.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-health.sh"

rm -f "$SHIMMER_RESULTS"
echo "nothing to undo: the health check only reads"
