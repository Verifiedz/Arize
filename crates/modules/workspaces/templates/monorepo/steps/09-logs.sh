# Detached: one terminal following every app's log, each under its app's name.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

[ "${LOGS_TERMINAL:-yes}" = "yes" ] || exit 0
[ "${TERMINAL_APP:-none}" = "none" ] && exit 0
ls "$SHIMMER_STATE_DIR"/dev-*.log >/dev/null 2>&1 || exit 0
shimmer_open_terminal "$TERMINAL_APP" "$SHIMMER_STATE_DIR" "tail -n 30 -F dev-*.log"
