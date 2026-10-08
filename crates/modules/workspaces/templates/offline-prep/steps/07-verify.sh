# Supervised: does each project really work offline? Each tool in its own offline mode (cargo
# --offline, GOPROXY=off, pip --no-index, uv --offline…) or a look at what's on disk (every
# dependency in node_modules), docker images present, and the services in its .env files that
# need the network whatever is downloaded.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

shimmer_offline_skipped verify && exit 0
while IFS= read -r p; do
    [ -n "$p" ] || continue
    name=$(basename "$p")
    failed=""
    checked=""
    why=""
    out="$SHIMMER_STATE_DIR/verify.out"
    for kind in $(shimmer_offline_kinds "$p"); do
        cmd=$(shimmer_offline_verify_command "$p" "$kind")
        [ -n "$cmd" ] || continue
        if shimmer_offline_run "$p" "$cmd" >"$out" 2>&1; then
            checked="$checked $kind"
        else
            failed="$failed $kind"
            # The tool's own last word on it, e.g. "missing from node_modules: left-pad".
            last=$(grep -v '^[[:space:]]*$' "$out" | tail -n 1 | cut -c 1-160)
            why="$why${why:+; }$kind: $last"
        fi
        cat "$out"
    done
    rm -f "$out"
    if shimmer_has docker && docker info </dev/null >/dev/null 2>&1; then
        while IFS= read -r f; do
            [ -n "$f" ] || continue
            for image in $(docker compose -f "$f" config --images </dev/null 2>/dev/null); do
                if docker image inspect "$image" </dev/null >/dev/null 2>&1; then
                    checked="$checked docker"
                else
                    failed="$failed docker($image)"
                fi
            done
        done <<COMPOSE
$(shimmer_offline_compose_files "$p")
COMPOSE
    fi
    checked=$(printf '%s\n' $checked | sort -u | tr '\n' ' ' | sed 's/ $//')
    if [ -n "$failed" ]; then
        shimmer_offline_result warn verify "$name: NOT ready offline ($why)"
    elif [ -n "$checked" ]; then
        shimmer_offline_result ok verify "$name: works offline ($checked)"
    else
        shimmer_offline_result info verify "$name: nothing this template can check offline"
    fi
    hosts=$(shimmer_offline_remote_hosts "$p")
    [ -z "$hosts" ] || shimmer_offline_result info verify "$name: its .env uses services that need the network: $hosts"
done <<PROJECTS
$(shimmer_offline_existing)
PROJECTS
exit 0
