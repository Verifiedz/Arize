# Detached: every app's dev server, each writing its own log in the temp folder, all in this
# step's process group, so stop stops them together. This step stays running while they do, so
# opening the workspace again doesn't start a second set.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-monorepo.sh"

[ -n "$(shimmer_dev_server_pid)" ] && exit 0
commands=$(shimmer_mono_commands "$PROJECT_DIR")
[ -n "$commands" ] || exit 0
mkdir -p "$SHIMMER_STATE_DIR"
rm -f "$SHIMMER_STATE_DIR"/dev-*.log "$SHIMMER_STATE_DIR"/dev-*.pid
echo $$ > "$SHIMMER_STATE_DIR/dev-server.pid"
tab=$(printf '\t')
# Not a pipeline: in sh a piped `while` runs in a subshell, and this step's `wait` would then have
# no children to wait for.
while IFS="$tab" read -r label cmd; do
    [ -n "$cmd" ] || continue
    echo "$ $cmd" >"$SHIMMER_STATE_DIR/dev-$label.log"
    (shimmer_run_in_project "$PROJECT_DIR" "$cmd") >>"$SHIMMER_STATE_DIR/dev-$label.log" 2>&1 &
    echo $! >"$SHIMMER_STATE_DIR/dev-$label.pid"
done <<COMMANDS
$commands
COMMANDS
wait
