# Supervised: every docker image the projects use: their compose files' images (pulled, then
# built where they build their own), and each Dockerfile's FROM images.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

shimmer_offline_skipped docker && exit 0
shimmer_has docker || exit 0
if ! docker info >/dev/null 2>&1; then
    shimmer_offline_result warn docker "docker isn't running, so no images were downloaded: start it and run this again"
    exit 0
fi
while IFS= read -r p; do
    [ -n "$p" ] || continue
    name=$(basename "$p")
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        if shimmer_offline_run "$p" "docker compose -f $(shimmer_quote "$f") pull --ignore-buildable && docker compose -f $(shimmer_quote "$f") build"; then
            shimmer_offline_result ok docker "$name: $(basename "$f") images ready"
        else
            shimmer_offline_result warn docker "$name: $(basename "$f") images failed (see the docker step's log)"
        fi
    done <<COMPOSE
$(shimmer_offline_compose_files "$p")
COMPOSE
    for df in "$p"/Dockerfile "$p"/*.Dockerfile; do
        [ -f "$df" ] || continue
        for image in $(awk 'toupper($1) == "FROM" { for (i = 2; i <= NF; i++) if ($i !~ /^--/) { print $i; break } }' "$df" | grep -v '\$' | sort -u); do
            # A FROM naming an earlier stage of the same file isn't an image to download.
            grep -qiE "^FROM .* AS $image\$" "$df" && continue
            if shimmer_offline_run "$p" "docker pull -q $(shimmer_quote "$image")"; then
                shimmer_offline_result ok docker "$name: base image $image"
            else
                shimmer_offline_result warn docker "$name: couldn't pull base image $image"
            fi
        done
    done
done <<PROJECTS
$(shimmer_offline_existing)
PROJECTS
exit 0
