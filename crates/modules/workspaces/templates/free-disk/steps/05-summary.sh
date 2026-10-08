# Supervised: the one-screen answer, biggest first. `workspaces activate --wait` prints this.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-free.sh"

[ -f "$SHIMMER_RESULTS" ] || : >"$SHIMMER_RESULTS"
if shimmer_free_preview; then verb=would; else verb=freed; fi
# "freed 123 label" lines, biggest first; at most 15, with the rest counted.
lines=$(grep "^$verb " "$SHIMMER_RESULTS" | grep -v "^$verb ? " | sort -k2,2nr)
count=$(printf '%s\n' "$lines" | awk '$2 > 0' | grep -c .)
printf '%s\n' "$lines" | head -n 15 | while read -r _ kb label; do
    [ "$kb" -gt 0 ] || continue
    if [ "$verb" = would ]; then
        echo "would free $(shimmer_free_human "$kb"): $label"
    else
        echo "freed $(shimmer_free_human "$kb"): $label"
    fi
done
[ "$count" -gt 15 ] && echo "… and $((count - 15)) more"
empty=$(printf '%s\n' "$lines" | awk '$2 == 0 { $1 = ""; $2 = ""; sub(/^  /, ""); printf "%s%s", sep, $0; sep = ", " }')
[ -z "$empty" ] || echo "nothing to free: $empty"
grep "^would ? " "$SHIMMER_RESULTS" | sed 's/^would ? /would also run: /'
grep -e '^skipped ' -e '^failed ' "$SHIMMER_RESULTS"

total=$(printf '%s\n' "$lines" | awk '{ s += $2 } END { print s + 0 }')
before=$(cat "$SHIMMER_STATE_DIR/avail-before" 2>/dev/null)
if [ "$verb" = would ]; then
    echo "in all, would free $(shimmer_free_human "$total") (MODE = preview: nothing was removed)"
    echo "to remove them: shimmer workspaces reconfigure $SHIMMER_WORKSPACE_ID --set MODE=clean, then activate it again"
else
    echo "in all, freed $(shimmer_free_human "$total"); free space now $(shimmer_free_human "$(shimmer_free_avail "$HOME")") (was $(shimmer_free_human "$before"))"
fi
exit 0
