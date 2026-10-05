# Detached: keeps running after the launch, the way an editor or a dev server would. Its
# process id is saved so cleanup can stop it.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

mkdir -p "$SHIMMER_STATE_DIR"
echo $$ > "$SHIMMER_STATE_DIR/background.pid"
exec sleep 300
