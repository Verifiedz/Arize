# Detached: a terminal in the folder that shows how to run the code.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-scratch.sh"

[ "${TERMINAL_APP:-none}" = "none" ] && exit 0
dir=$(shimmer_scratch_current)
[ -n "$dir" ] || exit 0
run=$(shimmer_scratch_run "${LANGUAGE:-python}")
hint=""
[ -n "$run" ] && hint="echo $(shimmer_quote "run it with: $run")"
shimmer_open_terminal "$TERMINAL_APP" "$dir" "$hint"
