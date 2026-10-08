# Supervised: the build folders of projects untouched for UNTOUCHED_DAYS days. Removed, not
# moved to the trash (that would free nothing): `cargo build` or `npm install` makes them again.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-free.sh"

tab=$(printf '\t')
folders=$(shimmer_free_build_folders)
# Not a pipeline: results are written by this shell (a piped `while` is a subshell).
while IFS="$tab" read -r kind folder; do
    [ -n "$folder" ] || continue
    shimmer_free_skipped "$kind" && continue
    project=$(dirname "$folder")
    shimmer_free_untouched "$project" || continue
    size=$(shimmer_free_size "$folder")
    if shimmer_free_preview; then
        shimmer_free_result "would $size $folder"
    elif rm -rf "$folder"; then
        echo "removed $folder"
        shimmer_free_result "freed $size $folder"
    else
        shimmer_free_result "failed $folder (couldn't remove it all)"
    fi
done <<FOLDERS
$folders
FOLDERS
for kind in rust-targets node-modules; do
    shimmer_free_skipped "$kind" && shimmer_free_result "skipped $kind (in SKIP)"
done
exit 0
