# Supervised: start what the apps need first, e.g. databases.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

[ -n "$SERVICES_COMMAND" ] || exit 0
shimmer_run_in_project "$PROJECT_DIR" "$SERVICES_COMMAND"
