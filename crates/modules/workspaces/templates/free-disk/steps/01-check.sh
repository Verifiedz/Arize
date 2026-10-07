# Supervised: say what will be cleaned, remember the free space now, and stop with one clear
# sentence if the answers can't work. Starts every run with a fresh summary.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-free.sh"

mkdir -p "$SHIMMER_STATE_DIR"
rm -f "$SHIMMER_RESULTS"
case "${MODE:-preview}" in clean | preview) ;; *) shimmer_fail "MODE must be clean or preview, not '$MODE'" ;; esac
case "${UNTOUCHED_DAYS:-30}" in
    *[!0-9]* | 0*) shimmer_fail "UNTOUCHED_DAYS must be a number of days from 1 up, not '$UNTOUCHED_DAYS'" ;;
esac
case "${SYSTEM_CLEAN:-skip}" in
    skip) ;;
    terminal)
        problem=$(shimmer_display_problem)
        [ -z "$problem" ] || shimmer_fail "$problem"
        shimmer_has_terminal "${TERMINAL_APP:-auto}" || shimmer_fail "the terminal '$TERMINAL_APP' isn't installed here"
        ;;
    passwordless) ;;
    *) shimmer_fail "SYSTEM_CLEAN must be skip, terminal or passwordless, not '$SYSTEM_CLEAN'" ;;
esac

dirs=$(shimmer_free_code_dirs | tr '\n' ' ')
echo "projects in: ${dirs:-none of CODE_DIRS exist ($CODE_DIRS)}"
caches=$(shimmer_free_caches | cut -f 1 | tr '\n' ' ')
echo "caches: ${caches:-none found}"
[ "${SYSTEM_CLEAN:-skip}" = "skip" ] || echo "system: $(shimmer_free_system_command "$SYSTEM_CLEAN")"
[ -n "$SKIP" ] && echo "never cleaned: $SKIP"
shimmer_free_preview && echo "preview: nothing will be removed"

shimmer_free_avail "$HOME" >"$SHIMMER_STATE_DIR/avail-before"
echo "free now: $(shimmer_free_human "$(cat "$SHIMMER_STATE_DIR/avail-before")")"
