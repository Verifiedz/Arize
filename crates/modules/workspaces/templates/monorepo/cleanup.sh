# Cleanup (supervised): stop every app's dev server and the services, then (CLOSE_ON_STOP) the
# windows it can close. Run by `workspaces stop`, and by `workspaces cleanup` after a failed
# launch. It lists what it closed and what it left open.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

pid=$(shimmer_dev_server_pid)
if [ -n "$pid" ]; then
    shimmer_stop_group "$pid"
    echo "stopped the dev servers ($pid)"
else
    echo "no dev servers running"
fi
rm -f "$SHIMMER_STATE_DIR/dev-server.pid" "$SHIMMER_STATE_DIR"/dev-*.pid

if [ -n "$SERVICES_STOP_COMMAND" ]; then
    shimmer_run_in_project "$PROJECT_DIR" "$SERVICES_STOP_COMMAND" || shimmer_fail "couldn't stop the services ($SERVICES_STOP_COMMAND)"
fi

shimmer_close_windows
