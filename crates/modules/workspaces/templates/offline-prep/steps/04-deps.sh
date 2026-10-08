# Supervised: download every project's dependencies with the tools it already uses. For Rust
# projects, says if rust-src or rust-analyzer (editor support offline) is missing; never adds it.
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

# Editor support for Rust offline needs rust-src and rust-analyzer. Your toolchain is yours:
# say what's missing, never add it.
if [ "$rust_parts" = yes ] && shimmer_has rustup; then
    installed=$(rustup component list --installed 2>/dev/null)
    missing=""
    for part in rust-src rust-analyzer; do
        printf '%s\n' "$installed" | grep -q "^$part" || missing="$missing $part"
    done
    if [ -n "$missing" ]; then
        shimmer_offline_result info deps "for Rust editor support offline, add:$missing (rustup component add$missing)"
    fi
fi
exit 0
