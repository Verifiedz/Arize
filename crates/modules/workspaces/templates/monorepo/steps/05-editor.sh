# Detached: the editor on the monorepo root, with a terminal inside it running
# IDE_TERMINAL_COMMAND (VS Code and Cursor).
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

case "${CODE_EDITOR:-none}" in
    vscode | cursor) shimmer_ide_terminal "$PROJECT_DIR" "$IDE_TERMINAL_COMMAND" ;;
    *) [ -z "$IDE_TERMINAL_COMMAND" ] || echo "IDE_TERMINAL_COMMAND only works with vscode and cursor" ;;
esac
shimmer_open_editor "${CODE_EDITOR:-none}" "$PROJECT_DIR"
[ "${CODE_EDITOR:-none}" = "none" ] || [ "$CODE_EDITOR" = "neovim" ] ||
    shimmer_record_left_open "the $CODE_EDITOR window (the editor keeps all its windows in one program, and may hold unsaved work)"
