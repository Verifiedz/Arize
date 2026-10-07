# Supervised: the report, needs-attention first, and the start page OFFLINE_DIR/index.html that
# links everything saved. `workspaces activate --wait` prints this.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

[ -f "$SHIMMER_RESULTS" ] || : >"$SHIMMER_RESULTS"
tab=$(printf '\t')
offline=$(shimmer_offline_dir)
mkdir -p "$offline"

html() {
    printf '%s' "$1" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g'
}

# ---- the start page
{
    echo '<!doctype html><html lang="en"><head><meta charset="utf-8">'
    echo '<meta name="viewport" content="width=device-width, initial-scale=1"><title>Offline</title>'
    echo '<style>body{font:16px/1.5 system-ui,sans-serif;max-width:52rem;margin:2rem auto;padding:0 1rem}'
    echo 'h2{margin-top:2rem}li{margin:.2rem 0}.warn{color:#b45309}.ok{color:#15803d}code{font-size:.9em}</style></head><body>'
    echo "<h1>Offline</h1><p>Prepared $(date '+%Y-%m-%d %H:%M') by Shimmer's offline-prep workspace.</p>"
    echo '<h2>Projects</h2><ul>'
    while IFS= read -r p; do
        [ -n "$p" ] || continue
        echo "<li><code>$(html "$p")</code>"
        # Rust docs of the project's own crates (cargo doc names folders with _ for -).
        if [ -d "$p/target/doc" ]; then
            for crate in $(find "$p" -maxdepth 3 -name Cargo.toml -not -path '*/target/*' -exec sed -n 's/^name *= *"\(.*\)"/\1/p' {} \; 2>/dev/null | tr '-' '_' | sort -u); do
                [ -f "$p/target/doc/$crate/index.html" ] && echo " · <a href=\"file://$(html "$p/target/doc/$crate/index.html")\">$crate docs</a>"
            done
        fi
        echo '</li>'
    done <<PROJECTS
$(shimmer_offline_existing)
PROJECTS
    echo '</ul>'
    if [ -d "$offline/pages" ]; then
        echo '<h2>Saved pages</h2><ul>'
        for f in "$offline/pages"/*.html; do
            [ -f "$f" ] && echo "<li><a href=\"pages/$(html "$(basename "$f")")\">$(html "$(basename "$f" .html)")</a></li>"
        done
        echo '</ul>'
    fi
    if [ -d "$offline/github" ]; then
        echo '<h2>GitHub</h2><ul>'
        for f in "$offline/github"/*.md; do
            [ -f "$f" ] || continue
            b=$(basename "$f" .md)
            echo "<li><a href=\"github/$(html "$b").md\">$(html "$b")</a>$([ -f "$offline/github/$b.diff" ] && echo " · <a href=\"github/$(html "$b").diff\">diff</a>")</li>"
        done
        echo '</ul>'
    fi
    echo '<h2>Also offline</h2><ul>'
    echo '<li><code>rustup doc</code>: the Rust book and standard library</li>'
    echo '<li><code>python3 -m pydoc -b</code>: docs of every installed Python package</li>'
    echo '<li><code>go doc &lt;package&gt;</code>, <code>tldr &lt;command&gt;</code>, <code>man &lt;command&gt;</code></li>'
    [ -n "$AI_MODEL" ] && echo "<li><code>ollama run $(html "$AI_MODEL")</code>: your local AI</li>"
    echo '</ul><h2>Report</h2><ul>'
    for kind in warn ok info; do
        grep "^$kind$tab" "$SHIMMER_RESULTS" | cut -f 3- | while IFS= read -r line; do
            echo "<li class=\"$kind\">$(html "$line")</li>"
        done
    done
    echo '</ul></body></html>'
} >"$offline/index.html"

# ---- the report: every warning, each project's verdict, the rest counted (all of it is on the
# start page).
grep "^warn$tab" "$SHIMMER_RESULTS" | cut -f 3- | sed 's/^/! /'
grep "^ok${tab}verify$tab" "$SHIMMER_RESULTS" | cut -f 3- | sed 's/^/✓ /'
grep "^ok${tab}machine$tab" "$SHIMMER_RESULTS" | cut -f 3- | sed 's/^/✓ /'
done_parts=""
for part in repos deps docker build docs pages github ai; do
    n=$(grep -c "^ok${tab}$part$tab" "$SHIMMER_RESULTS")
    [ "$n" -gt 0 ] && done_parts="$done_parts, $part ($n)"
done
[ -z "$done_parts" ] || echo "✓ also done: ${done_parts#, }"
grep "^info${tab}verify$tab" "$SHIMMER_RESULTS" | cut -f 3- | grep 'need the network' | sed 's/^/· /'
ready=$(grep -c "^ok${tab}verify$tab" "$SHIMMER_RESULTS")
notready=$(grep -c "^warn${tab}verify$tab" "$SHIMMER_RESULTS")
warnings=$(grep -c "^warn$tab" "$SHIMMER_RESULTS")
echo "$ready ready offline, $notready not; $(shimmer_offline_count "$warnings" thing) to look at"
echo "everything, and the full report: file://$offline/index.html"
exit 0
