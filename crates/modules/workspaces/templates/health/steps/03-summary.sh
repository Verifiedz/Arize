# Supervised: the report, needs-attention first. `workspaces activate --wait` prints this, and
# with NOTIFY = on-warning a desktop notification says what needs attention.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-health.sh"

[ -f "$SHIMMER_RESULTS" ] || : >"$SHIMMER_RESULTS"
tab=$(printf '\t')
grep "^warn$tab" "$SHIMMER_RESULTS" | cut -f 2- | sed 's/^/! /'
grep "^ok$tab" "$SHIMMER_RESULTS" | cut -f 2- | sed 's/^/✓ /'
grep "^info$tab" "$SHIMMER_RESULTS" | cut -f 2- | sed 's/^/· /'
warnings=$(grep -c "^warn$tab" "$SHIMMER_RESULTS")
if [ "$warnings" -eq 0 ]; then
    echo "all good"
    exit 0
fi
if [ "$warnings" -eq 1 ]; then echo "1 thing needs attention"; else echo "$warnings things need attention"; fi

if [ "${NOTIFY:-never}" = "on-warning" ]; then
    body=$(grep "^warn$tab" "$SHIMMER_RESULTS" | cut -f 2- | head -n 3)
    if shimmer_macos; then
        text=$(printf '%s' "$body" | tr '\n' ';' | sed 's/\\/\\\\/g; s/"/\\"/g')
        osascript -e "display notification \"$text\" with title \"Shimmer health: $warnings to look at\"" >/dev/null 2>&1
    elif shimmer_has notify-send; then
        notify-send "Shimmer health: $warnings to look at" "$body" >/dev/null 2>&1
    else
        echo "(no notify-send here, so no notification)"
    fi
fi
exit 0
