# Supervised: say what this run will update, and stop with one clear sentence if the system
# update can't happen the way SYSTEM_UPDATES says. Starts every run with a fresh summary.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-update.sh"

mkdir -p "$SHIMMER_STATE_DIR"
rm -f "$SHIMMER_RESULTS" "$SHIMMER_STATE_DIR/system.exit" "$SHIMMER_STATE_DIR/system.pid" "$SHIMMER_STATE_DIR/closable"

case "${SYSTEM_UPDATES:-terminal}" in
    skip) echo "system: skipped (SYSTEM_UPDATES = skip)" ;;
    terminal | passwordless)
        tool=$(shimmer_update_system_tool)
        if [ -z "$tool" ]; then
            echo "system: no package manager this template knows (add yours to EXTRA_COMMANDS)"
        else
            echo "system: $(shimmer_update_system_command "$tool" "$SYSTEM_UPDATES")"
            if [ "$SYSTEM_UPDATES" = "terminal" ]; then
                problem=$(shimmer_display_problem)
                [ -z "$problem" ] || shimmer_fail "$problem"
                shimmer_has_terminal "${TERMINAL_APP:-auto}" || shimmer_fail "the terminal '$TERMINAL_APP' isn't installed here"
            fi
        fi
        ;;
    *) shimmer_fail "SYSTEM_UPDATES must be terminal, passwordless or skip, not '$SYSTEM_UPDATES'" ;;
esac

if shimmer_update_use_topgrade; then
    shimmer_has topgrade || shimmer_fail "USE_TOPGRADE is yes, but topgrade isn't installed"
    echo "tools: topgrade"
else
    tools=$(shimmer_update_tools | cut -f 1 | tr '\n' ' ')
    echo "tools: ${tools:-none of the ones this template knows are installed}"
fi
[ -n "$SKIP" ] && echo "never run: $SKIP"
[ -n "$EXTRA_COMMANDS" ] && echo "your commands: $(printf '%s\n' "$EXTRA_COMMANDS" | grep -c .)"
exit 0
