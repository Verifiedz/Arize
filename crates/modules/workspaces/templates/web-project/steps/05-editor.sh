# Detached: the editor on the project folder, or on OPEN_PATH inside it, with a terminal inside
# it running IDE_TERMINAL_COMMAND (VS Code and Cursor).
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

target=$PROJECT_DIR
case "$OPEN_PATH" in
    "") ;;
    /*) target=$OPEN_PATH ;;
    *) target="$PROJECT_DIR/$OPEN_PATH" ;;
esac
# A terminal inside the editor (VS Code and Cursor), before the editor opens the folder.
case "${CODE_EDITOR:-none}" in
    vscode | cursor) shimmer_ide_terminal "$PROJECT_DIR" "$IDE_TERMINAL_COMMAND" ;;
    *) [ -z "$IDE_TERMINAL_COMMAND" ] || echo "IDE_TERMINAL_COMMAND only works with vscode and cursor" ;;
esac
shimmer_open_editor "${CODE_EDITOR:-none}" "$target"
