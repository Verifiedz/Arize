# Detached: keeps running after the launch, the way an editor or a dev server would. Its
# process id is saved so cleanup can stop it.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

mkdir -p "$SHIMMER_STATE_DIR"
# Already running from an earlier activate: one is enough, and a second would be left behind
# when stop reads the pid file.
file="$SHIMMER_STATE_DIR/background.pid"
[ -f "$file" ] && kill -s 0 "$(cat "$file")" 2>/dev/null && exit 0
echo $$ > "$file"
exec sleep 300
