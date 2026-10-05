# Supervised: install packages, with `auto` only when node_modules is missing or older than the
# lockfile, so most launches skip it.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

[ -n "$INSTALL_COMMAND" ] || exit 0
if [ "$INSTALL_COMMAND" != "auto" ]; then
    shimmer_run_in_project "$PROJECT_DIR" "$INSTALL_COMMAND"
    exit
fi
pm=$(shimmer_package_manager "$PROJECT_DIR")
[ -n "$pm" ] || exit 0
lock="$PROJECT_DIR/$(shimmer_lockfile "$pm" "$PROJECT_DIR")"
modules="$PROJECT_DIR/node_modules"
if [ -d "$modules" ] && { [ ! -f "$lock" ] || [ ! "$lock" -nt "$modules" ]; }; then
    echo "packages are up to date"
    exit 0
fi
shimmer_run_in_project "$PROJECT_DIR" "$pm install" || exit 1
# Mark them installed, so the next launch skips this.
touch "$modules"
