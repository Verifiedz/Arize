# Supervised: make today's scratch folder with its starter file, and move old ones to the trash
# (KEEP_DAYS). The folder's path goes to the temp folder for the steps after this one.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-scratch.sh"

root=$(shimmer_scratch_root)
mkdir -p "$root" || shimmer_fail "can't make $root"

# Old ones first, so today's is never a candidate. Only folders this template made.
if [ -n "$KEEP_DAYS" ]; then
    for old in "$root"/*/; do
        old=${old%/}
        [ -f "$old/.shimmer-scratch" ] || continue
        # Anything inside changed within KEEP_DAYS days: still in use.
        [ -z "$(find "$old" -mtime "-$KEEP_DAYS" 2>/dev/null | head -n 1)" ] || continue
        if shimmer_trash "$old" 2>/dev/null; then
            echo "moved to the trash (untouched for $KEEP_DAYS days): $old"
        else
            echo "untouched for $KEEP_DAYS days, but there's no trash command here (gio, trash-put, trash), so kept: $old"
        fi
    done
fi

name="$(date +%Y-%m-%d)-$(shimmer_scratch_label "${LANGUAGE:-python}")"
dir="$root/$name"
if [ "$NEW_FOLDER" != "once-a-day" ]; then
    n=2
    while [ -e "$dir" ]; do
        dir="$root/$name-$n"
        n=$((n + 1))
    done
fi
fresh=no
[ -d "$dir" ] || fresh=yes
mkdir -p "$dir" || shimmer_fail "can't make $dir"
echo "made by Shimmer's scratch template: with KEEP_DAYS set, it goes to the trash once nothing in it has changed for that long" >"$dir/.shimmer-scratch"
shimmer_scratch_starter "${LANGUAGE:-python}" "$dir"
if [ "$GIT_INIT" = "yes" ] && [ ! -d "$dir/.git" ]; then
    git -C "$dir" init -q && echo ".shimmer-scratch" >>"$dir/.git/info/exclude"
fi

mkdir -p "$SHIMMER_STATE_DIR"
echo "$dir" >"$SHIMMER_STATE_DIR/current"
if [ "$fresh" = "yes" ]; then echo "made: $dir"; else echo "reopened: $dir"; fi
