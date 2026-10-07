# Detached: the editor on the folder, with the starter file open.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-scratch.sh"

dir=$(shimmer_scratch_current)
[ -n "$dir" ] || exit 0
main=$(shimmer_scratch_main "${LANGUAGE:-none}")
cmd=$(shimmer_editor_command "${CODE_EDITOR:-none}")
case "${CODE_EDITOR:-none}" in
    # These take the folder and a file in one go: the folder's window, with the file open.
    vscode | cursor | zed | sublime)
        if [ -n "$cmd" ] && [ -n "$main" ]; then
            "$cmd" "$dir" "$dir/$main" >/dev/null 2>&1 &
        else
            shimmer_open_editor "$CODE_EDITOR" "$dir"
        fi
        ;;
    neovim) shimmer_open_editor neovim "$dir/${main:-.}" ;;
    *) shimmer_open_editor "${CODE_EDITOR:-none}" "$dir" ;;
esac
[ "${CODE_EDITOR:-none}" = "none" ] || [ "$CODE_EDITOR" = "neovim" ] ||
    shimmer_record_left_open "the $CODE_EDITOR window (the editor keeps all its windows in one program, and may hold unsaved work)"
