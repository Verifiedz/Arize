# Supervised: start what the project needs before its dev server, e.g. a database.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

[ -n "$SERVICES_COMMAND" ] || exit 0
shimmer_run_in_project "$PROJECT_DIR" "$SERVICES_COMMAND"
