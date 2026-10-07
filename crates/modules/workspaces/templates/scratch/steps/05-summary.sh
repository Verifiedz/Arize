# Supervised: where the folder is and how to run it. `workspaces activate --wait` prints this.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-scratch.sh"

dir=$(shimmer_scratch_current)
echo "folder: $dir"
run=$(shimmer_scratch_run "${LANGUAGE:-python}")
if [ -n "$run" ]; then
    echo "run it: cd $(shimmer_quote "$dir") && $run"
elif [ "$(shimmer_scratch_lang "${LANGUAGE:-python}")" = "file" ]; then
    echo "to have it say how to run your code, set RUN_COMMAND: shimmer workspaces edit $SHIMMER_WORKSPACE_ID"
fi
exit 0
