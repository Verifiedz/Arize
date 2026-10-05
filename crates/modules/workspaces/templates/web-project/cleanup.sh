# Cleanup (supervised): stop what this workspace started: its dev server (freeing the port), then
# its services. Run by `workspaces stop`, and by `workspaces cleanup` after a failed launch.
# Windows (editor, browser, terminal) are never closed: they may hold your other work.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

pid=$(shimmer_dev_server_pid)
if [ -n "$pid" ]; then
    shimmer_stop_group "$pid"
    echo "stopped the dev server ($pid)"
else
    echo "no dev server running"
fi
rm -f "$SHIMMER_STATE_DIR/dev-server.pid"

if [ -n "$SERVICES_STOP_COMMAND" ]; then
    shimmer_run_in_project "$PROJECT_DIR" "$SERVICES_STOP_COMMAND" || shimmer_fail "couldn't stop the services ($SERVICES_STOP_COMMAND)"
fi
