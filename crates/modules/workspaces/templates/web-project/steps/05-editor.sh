# Detached: the editor on the project folder, or on OPEN_PATH inside it.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

target=$PROJECT_DIR
case "$OPEN_PATH" in
    "") ;;
    /*) target=$OPEN_PATH ;;
    *) target="$PROJECT_DIR/$OPEN_PATH" ;;
esac
shimmer_open_editor "${CODE_EDITOR:-none}" "$target"
