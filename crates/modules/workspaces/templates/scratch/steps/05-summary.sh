# Supervised: where the folder is and how to run it. `workspaces activate --wait` prints this.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-scratch.sh"

dir=$(shimmer_scratch_current)
echo "folder: $dir"
run=$(shimmer_scratch_run "${LANGUAGE:-none}")
[ -n "$run" ] && echo "run it: cd $(shimmer_quote "$dir") && $run"
exit 0
