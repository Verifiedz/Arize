# Detached: another terminal in the repo, running TERMINAL_COMMAND (e.g. claude).
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

[ -n "$TERMINAL_COMMAND" ] || exit 0
[ "${TERMINAL_APP:-none}" = "none" ] && exit 0
shimmer_open_terminal "$TERMINAL_APP" "$PROJECT_DIR" "$TERMINAL_COMMAND"
