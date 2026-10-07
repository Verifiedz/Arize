# Supervised: stop with one clear sentence before anything opens.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-scratch.sh"

problem=$(shimmer_display_problem)
[ -z "$problem" ] || shimmer_fail "$problem"
case "${LANGUAGE:-none}" in
    python | rust | javascript | typescript | go | c | cpp | java | shell | none) ;;
    *) shimmer_fail "LANGUAGE '$LANGUAGE' isn't one this template knows (python, rust, javascript, typescript, go, c, cpp, java, shell, none)" ;;
esac
case "$KEEP_DAYS" in
    "") ;;
    *[!0-9]* | 0*) shimmer_fail "KEEP_DAYS must be a number of days from 1 up, or empty to keep everything" ;;
esac
shimmer_has_editor "${CODE_EDITOR:-none}" || shimmer_fail "the editor '$CODE_EDITOR' isn't installed here"
shimmer_has_terminal "${TERMINAL_APP:-none}" || shimmer_fail "the terminal '$TERMINAL_APP' isn't installed here"
[ "$GIT_INIT" = "yes" ] && { shimmer_has git || shimmer_fail "GIT_INIT is yes, but git isn't installed"; }

program=$(shimmer_scratch_program "${LANGUAGE:-none}")
if [ -n "$program" ] && ! shimmer_has "$program"; then
    echo "note: $program isn't installed, so the code won't run yet (you can still write it)"
fi
case "$(shimmer_scratch_root)" in
    /*) ;;
    *) shimmer_fail "SCRATCH_DIR must be a full path like ~/scratch or /home/you/scratch, not '$SCRATCH_DIR'" ;;
esac
echo "ok: ${LANGUAGE:-none} in $(shimmer_scratch_root)"
