# Detached: the dev server, logging to the temp folder. Its process id is saved so cleanup can
# stop it, and so opening the workspace again doesn't start a second one.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

dev=$(shimmer_dev_command "$PROJECT_DIR" "$DEV_COMMAND")
[ -n "$dev" ] || exit 0
[ -n "$(shimmer_dev_server_pid)" ] && exit 0
mkdir -p "$SHIMMER_STATE_DIR"
echo $$ > "$SHIMMER_STATE_DIR/dev-server.pid"
shimmer_run_in_project "$PROJECT_DIR" "$dev" > "$SHIMMER_STATE_DIR/dev-server.log" 2>&1
