# Supervised: the system's downloaded packages and old journal logs, which need sudo. With
# SYSTEM_CLEAN = terminal a window opens and you type your password into sudo there; Shimmer
# never sees it. Measured by the free space on /.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-free.sh"

mode=${SYSTEM_CLEAN:-skip}
[ "$mode" = "skip" ] && exit 0
cmd=$(shimmer_free_system_command "$mode")
if [ -z "$cmd" ]; then
    shimmer_free_result "skipped system (no package manager or journal this template knows)"
    exit 0
fi
if shimmer_free_preview; then
    shimmer_free_result "would ? system ($cmd)"
    exit 0
fi

before=$(shimmer_free_avail /)
if [ "$mode" = "passwordless" ]; then
    echo "== system: $cmd"
    out="$SHIMMER_STATE_DIR/system.out"
    sh -c "$cmd" </dev/null >"$out" 2>&1
    code=$?
    cat "$out"
    grep -q "password is required" "$out" && code=253
    rm -f "$out"
else
    shimmer_terminal_and_wait system "$cmd"
    code=$?
fi
freed=$(($(shimmer_free_avail /) - before))
[ "$freed" -gt 0 ] || freed=0
case "$code" in
    0) shimmer_free_result "freed $freed system package cache and logs" ;;
    253) shimmer_free_result "failed system: sudo wants a password; add a sudo rule for it or use SYSTEM_CLEAN = terminal" ;;
    254) shimmer_free_result "failed system: its window was closed before it finished" ;;
    *) shimmer_free_result "failed system (exit $code)" ;;
esac
exit 0
