# Supervised: with WARM_BUILD = yes, compile each project once (with its tests), offline, so the
# first build on the plane is quick and the dependencies are proven to compile.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

[ "${WARM_BUILD:-no}" = "yes" ] || exit 0
shimmer_offline_skipped build && exit 0
while IFS= read -r p; do
    [ -n "$p" ] || continue
    name=$(basename "$p")
    for kind in $(shimmer_offline_kinds "$p"); do
        cmd=$(shimmer_offline_build_command "$p" "$kind")
        [ -n "$cmd" ] || continue
        if shimmer_offline_run "$p" "$cmd"; then
            shimmer_offline_result ok build "$name: $kind compiled"
        else
            shimmer_offline_result warn build "$name: $kind didn't compile ($cmd; see the build step's log)"
        fi
    done
done <<PROJECTS
$(shimmer_offline_existing)
PROJECTS
exit 0
