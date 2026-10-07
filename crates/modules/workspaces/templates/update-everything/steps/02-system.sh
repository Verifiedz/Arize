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

# terminal: the window records its process id when it starts and its exit code when it ends.
pid_file="$SHIMMER_STATE_DIR/system.pid"
exit_file="$SHIMMER_STATE_DIR/system.exit"
inner="echo \$\$ > $(shimmer_quote "$pid_file"); echo '$ $cmd'; $cmd; code=\$?; echo \$code > $(shimmer_quote "$exit_file")"
inner="$inner; echo; echo 'Done: this window closes in 10 seconds.'; sleep 10; exit"
shimmer_open_terminal "${TERMINAL_APP:-auto}" "$HOME" "$inner"
echo "opened a terminal for: $cmd"

waited=0
while [ ! -f "$exit_file" ]; do
    sleep 1
    waited=$((waited + 1))
    if [ -f "$pid_file" ]; then
        if ! kill -s 0 "$(cat "$pid_file")" 2>/dev/null && [ ! -f "$exit_file" ]; then
            shimmer_update_result "failed system ($tool): its window was closed before it finished"
            exit 0
        fi
    elif [ "$waited" -ge 30 ]; then
        shimmer_fail "the terminal for the system update didn't open within 30 seconds"
    fi
done
code=$(cat "$exit_file")
if [ "$code" = "0" ]; then
    shimmer_update_result "updated system ($tool)"
else
    shimmer_update_result "failed system ($tool) (exit $code, see its window)"
fi
