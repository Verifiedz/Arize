# Supervised: the one-screen answer. `workspaces activate --wait` prints this step's log.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-update.sh"

if [ ! -s "$SHIMMER_RESULTS" ]; then
    echo "nothing to update"
    exit 0
fi
for kind in updated skipped failed; do
    grep "^$kind " "$SHIMMER_RESULTS"
done
failed=$(grep -c '^failed ' "$SHIMMER_RESULTS")
updated=$(grep -c '^updated ' "$SHIMMER_RESULTS")
echo "$updated updated, $failed failed"
if [ "$failed" -gt 0 ]; then
    logs="$SHIMMER_HOME/logs/$SHIMMER_WORKSPACE_ID-$SHIMMER_SESSION_ID"
    echo "full output: $logs-tools.log (and $logs-system.log)"
fi
exit 0
