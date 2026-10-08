# Supervised: the inbox, what's waiting on you first. `workspaces activate --wait` prints this,
# and with NOTIFY = when-reviews a desktop notification says how many reviews are waiting.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-github.sh"

[ -f "$SHIMMER_RESULTS" ] || : >"$SHIMMER_RESULTS"
tab=$(printf '\t')
stale=${STALE_DAYS:-2}

# One section: its heading with a count, then "  repo#n  title — detail" lines.
section() {
    lines=$(grep "^$1$tab" "$SHIMMER_RESULTS")
    count=$(printf '%s\n' "$lines" | grep -c .)
    if [ "$count" -eq 0 ]; then
        echo "$2: none"
        return
    fi
    echo "$2 ($count):"
    printf '%s\n' "$lines" | while IFS="$tab" read -r _ ref title detail url days; do
        mark=" "
        [ "$1" = "review" ] && [ -n "$days" ] && [ "$days" -ge "$stale" ] && mark="!"
        # printf, not echo: dash's echo would turn a "\t" in a title into a tab.
        printf '%s %s  %s — %s\n' "$mark" "$ref" "$title" "$detail"
    done
}

section review "Waiting for your review"
section mine "Your open PRs"
section issue "Assigned to you"
unread=$(cat "$SHIMMER_STATE_DIR/unread" 2>/dev/null)
[ -n "$unread" ] && echo "Notifications: $unread unread — $(shimmer_github_web)/notifications"

reviews=$(grep -c "^review$tab" "$SHIMMER_RESULTS")
if [ "${NOTIFY:-never}" = "when-reviews" ] && [ "$reviews" -gt 0 ]; then
    first=$(grep "^review$tab" "$SHIMMER_RESULTS" | head -n 3 | cut -f 2,3 | tr '\t' ' ')
    title="GitHub: $reviews PR$([ "$reviews" -gt 1 ] && echo s) waiting for your review"
    if shimmer_macos; then
        text=$(printf '%s' "$first" | tr '\n' ';' | sed 's/\\/\\\\/g; s/"/\\"/g')
        osascript -e "display notification \"$text\" with title \"$title\"" >/dev/null 2>&1
    elif shimmer_has notify-send; then
        notify-send "$title" "$first" >/dev/null 2>&1
    fi
fi
exit 0
