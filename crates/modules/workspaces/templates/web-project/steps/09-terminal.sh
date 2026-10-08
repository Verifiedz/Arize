# Detached: a terminal in the project, running TERMINAL_COMMAND (e.g. claude) if set.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

[ "${TERMINAL_APP:-none}" = "none" ] && exit 0
shimmer_open_terminal "$TERMINAL_APP" "$PROJECT_DIR" "$TERMINAL_COMMAND"
