# Supervised: the one-screen answer. `workspaces activate --wait` prints this step's log.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-update.sh"

if [ ! -s "$SHIMMER_RESULTS" ]; then
    echo "nothing to update"
    exit 0
fi
if grep -q '^would ' "$SHIMMER_RESULTS"; then
    echo "MODE = preview: nothing ran. update would run:"
    grep '^would ' "$SHIMMER_RESULTS" | sed 's/^would /  /'
    grep '^skipped ' "$SHIMMER_RESULTS"
    echo "to run them: shimmer workspaces reconfigure $SHIMMER_WORKSPACE_ID --set MODE=update, then activate it again"
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
