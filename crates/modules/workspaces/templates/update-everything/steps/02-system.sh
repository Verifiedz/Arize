# Supervised: update the system's packages, which needs sudo. With SYSTEM_UPDATES = terminal a
# window opens and you type your password into sudo there; Shimmer never sees it. This step
# waits until that window's update finishes, or the window is closed.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-update.sh"

mode=${SYSTEM_UPDATES:-terminal}
[ "$mode" = "skip" ] && exit 0
tool=$(shimmer_update_system_tool)
[ -n "$tool" ] || exit 0
cmd=$(shimmer_update_system_command "$tool" "$mode")

if [ "$mode" = "passwordless" ]; then
    echo "== system ($tool): $cmd"
    out="$SHIMMER_STATE_DIR/system.out"
    sh -c "$cmd" </dev/null >"$out" 2>&1
    code=$?
    cat "$out"
    if [ "$code" -eq 0 ]; then
        shimmer_update_result "updated system ($tool)"
    elif grep -q "password is required" "$out"; then
        shimmer_update_result "failed system ($tool): sudo wants a password; add a sudo rule for it (see workspace.toml) or use SYSTEM_UPDATES = terminal"
    else
        shimmer_update_result "failed system ($tool) (exit $code)"
    fi
    rm -f "$out"
    exit 0
fi

# terminal: you type your password into sudo in that window.
shimmer_terminal_and_wait system "$cmd"
code=$?
case "$code" in
    0) shimmer_update_result "updated system ($tool)" ;;
    254) shimmer_update_result "failed system ($tool): its window was closed before it finished" ;;
    *) shimmer_update_result "failed system ($tool) (exit $code, see its window)" ;;
esac
