# Supervised: download every project's dependencies with the tools it already uses. Rust
# projects also get rust-src and rust-analyzer, which editors need offline.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

shimmer_offline_skipped deps && exit 0
rust_parts=no
while IFS= read -r p; do
    [ -n "$p" ] || continue
    name=$(basename "$p")
    kinds=$(shimmer_offline_kinds "$p")
    if [ -z "$kinds" ]; then
        shimmer_offline_result info deps "$name: no dependencies this template recognises"
        continue
    fi
    for kind in $kinds; do
        cmd=$(shimmer_offline_fetch_command "$p" "$kind")
        if [ -z "$cmd" ]; then
            shimmer_offline_result warn deps "$name: uses $kind, but its tool isn't installed here"
            continue
        fi
        if shimmer_offline_run "$p" "$cmd"; then
            shimmer_offline_result ok deps "$name: $kind dependencies downloaded"
        else
            shimmer_offline_result warn deps "$name: $kind dependencies failed ($cmd; see the deps step's log)"
        fi
        [ "$kind" = rust ] && rust_parts=yes
    done
done <<PROJECTS
$(shimmer_offline_existing)
PROJECTS

if [ "$rust_parts" = yes ] && shimmer_has rustup; then
    echo "== rustup component add rust-src rust-analyzer"
    if rustup component add rust-src rust-analyzer </dev/null; then
        shimmer_offline_result ok deps "rust-src and rust-analyzer installed (your editor's Rust support works offline)"
    else
        shimmer_offline_result warn deps "couldn't add rust-src and rust-analyzer"
    fi
fi
exit 0
