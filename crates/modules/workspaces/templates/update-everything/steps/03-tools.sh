# Supervised: update every installed developer tool (or let topgrade do it), then run your own
# EXTRA_COMMANDS, one line at a time. Each one's output is in this step's log.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-update.sh"

tab=$(printf '\t')
if shimmer_update_use_topgrade; then
    # The system is the system step's job: topgrade can't ask for a password here either.
    disable="system"
    for name in $SKIP; do disable="$disable $name"; done
    shimmer_update_run topgrade "topgrade --yes --no-retry --skip-notify --disable $disable"
else
    tools=$(shimmer_update_tools)
    # Not a pipeline: the results must be written by this shell (a piped `while` is a subshell).
    while IFS="$tab" read -r name cmd; do
        [ -n "$name" ] || continue
        if shimmer_update_skipped "$name"; then
            shimmer_update_result "skipped $name (in SKIP)"
            continue
        fi
        why=$(shimmer_update_needs_sudo "$name")
        if [ -n "$why" ]; then
            shimmer_update_result "skipped $name ($why; update it with sudo yourself)"
            continue
        fi
        shimmer_update_run "$name" "$cmd"
    done <<TOOLS
$tools
TOOLS
fi

# Your own commands, named by their first word.
while IFS= read -r cmd; do
    case "$cmd" in "" | \#*) continue ;; esac
    shimmer_update_run "$(printf '%s' "$cmd" | awk '{ print $1 }')" "$cmd"
done <<EXTRA
$EXTRA_COMMANDS
EXTRA
exit 0
