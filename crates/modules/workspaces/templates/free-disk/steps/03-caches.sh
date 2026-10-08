# Supervised: each installed tool's download cache, through the tool's own clean command. One
# failing never stops the others.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-free.sh"

cd "$HOME" || exit 0
tab=$(printf '\t')
caches=$(shimmer_free_caches)
while IFS="$tab" read -r name dir cmd; do
    [ -n "$name" ] && [ -n "$cmd" ] || continue
    # A tool that couldn't say where its cache is: measured by the disk instead.
    [ -n "$dir" ] && [ "$dir" != "-" ] || dir=""
    label="$name cache"
    [ "$name" = "docker" ] && label="docker (stopped containers, unused images, build cache)"
    if shimmer_free_skipped "$name"; then
        shimmer_free_result "skipped $name (in SKIP)"
        continue
    fi
    if shimmer_free_preview; then
        if [ -n "$dir" ]; then
            shimmer_free_result "would $(shimmer_free_size "$dir") $label"
        else
            shimmer_free_result "would ? $name ($cmd)"
        fi
        continue
    fi
    # Measured by the folder when there is one, else by the disk's free space.
    if [ -n "$dir" ]; then before=$(shimmer_free_size "$dir"); else before=$(shimmer_free_avail /); fi
    echo
    echo "== $name: $cmd"
    sh -c "$cmd" </dev/null
    code=$?
    if [ -n "$dir" ]; then freed=$((before - $(shimmer_free_size "$dir"))); else freed=$(($(shimmer_free_avail /) - before)); fi
    [ "$freed" -gt 0 ] || freed=0
    if [ "$code" -eq 0 ]; then
        shimmer_free_result "freed $freed $label"
    else
        shimmer_free_result "failed $name (exit $code)"
    fi
done <<CACHES
$caches
CACHES
exit 0
