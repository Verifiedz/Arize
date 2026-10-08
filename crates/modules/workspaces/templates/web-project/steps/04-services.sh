# Supervised: start what the project needs before its dev server, e.g. a database.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

[ -n "$SERVICES_COMMAND" ] || exit 0
# Started by an earlier activate and not stopped since: starting them twice would leave the
# first ones running with nothing to stop them.
started="$SHIMMER_STATE_DIR/services.started"
if [ -f "$started" ]; then
    echo "services already started by the last activate"
    exit 0
fi
shimmer_run_in_project "$PROJECT_DIR" "$SERVICES_COMMAND" || exit 1
mkdir -p "$SHIMMER_STATE_DIR" && touch "$started"
