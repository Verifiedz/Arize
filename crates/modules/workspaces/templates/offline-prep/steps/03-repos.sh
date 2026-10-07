# Supervised: clone what's missing, then for each repo: fetch, fast-forward when it's clean,
# submodules and Git LFS files, and say what's uncommitted or unpushed before you go.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

shimmer_offline_skipped repos && exit 0
shimmer_has git || exit 0

clone_dir=$(shimmer_offline_expand "${CLONE_DIR:-~/code}")
for repo in $CLONE_REPOS; do
    dest="$clone_dir/$(shimmer_offline_repo_name "$repo")"
    [ -d "$dest" ] && continue
    mkdir -p "$clone_dir"
    echo "== clone $repo"
    case "$repo" in
        *://* | *@*:*) git clone --recurse-submodules "$repo" "$dest" </dev/null ;;
        *) if shimmer_has gh; then gh repo clone "$repo" "$dest" -- --recurse-submodules; else git clone --recurse-submodules "https://github.com/$repo.git" "$dest"; fi </dev/null ;;
    esac
    if [ -d "$dest" ]; then
        shimmer_offline_result ok repos "cloned $repo into $dest"
    else
        shimmer_offline_result warn repos "couldn't clone $repo (see the repos step's log)"
    fi
done

while IFS= read -r p; do
    [ -n "$p" ] || continue
    name=$(basename "$p")
    git -C "$p" rev-parse --git-dir >/dev/null 2>&1 || continue
    echo
    echo "== $name"
    if ! git -C "$p" fetch --all --prune --quiet </dev/null; then
        shimmer_offline_result warn repos "$name: couldn't fetch (see the log)"
        continue
    fi
    dirty=$(git -C "$p" status --porcelain 2>/dev/null | grep -c .)
    branch=$(git -C "$p" symbolic-ref --short -q HEAD)
    note=""
    if [ "$dirty" -gt 0 ]; then
        note="fetched only: $(shimmer_offline_count "$dirty" "uncommitted file")"
    elif [ -n "$branch" ] && git -C "$p" rev-parse -q --verify '@{u}' >/dev/null; then
        if git -C "$p" merge --ff-only --quiet '@{u}' </dev/null; then
            note="up to date on $branch"
        else
            note="fetched only: $branch and its upstream have both moved"
        fi
    else
        note="fetched (no upstream branch)"
    fi
    if [ -f "$p/.gitmodules" ]; then
        git -C "$p" submodule update --init --recursive --quiet </dev/null || note="$note; submodules failed"
    fi
    if grep -qs 'filter=lfs' "$p/.gitattributes"; then
        if git lfs version >/dev/null 2>&1; then
            git -C "$p" lfs pull </dev/null || note="$note; LFS files failed"
        else
            note="$note; uses Git LFS but git-lfs isn't installed"
        fi
    fi
    ahead=$(git -C "$p" rev-list --count '@{u}..HEAD' 2>/dev/null || echo 0)
    if [ "$dirty" -gt 0 ] || [ "$ahead" -gt 0 ]; then
        msg="$name: $note"
        [ "$ahead" -gt 0 ] && msg="$msg; $(shimmer_offline_count "$ahead" commit) not pushed"
        shimmer_offline_result warn repos "$msg (commit and push before you go, so a lost laptop loses nothing)"
    else
        shimmer_offline_result ok repos "$name: $note"
    fi
done <<PROJECTS
$(shimmer_offline_existing)
PROJECTS
exit 0
