# Cleanup (supervised), when you're back online (`workspaces stop`): for each repo, fetch and
# say what's unpushed and what's behind. With PULL_ON_RETURN = yes, fast-forward clean repos;
# with PUSH_ON_RETURN = yes, push the current branch when it has commits its upstream doesn't.
# Never forced, never a merge. Nothing downloaded is removed (a free-disk workspace does that).
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

shimmer_has git || exit 0
while IFS= read -r p; do
    [ -n "$p" ] || continue
    git -C "$p" rev-parse --git-dir >/dev/null 2>&1 || continue
    name=$(basename "$p")
    if ! git -C "$p" fetch --all --prune --quiet </dev/null 2>/dev/null; then
        echo "$name: can't reach its remote yet (still offline?)"
        continue
    fi
    branch=$(git -C "$p" symbolic-ref --short -q HEAD)
    if [ -z "$branch" ] || ! git -C "$p" rev-parse -q --verify '@{u}' >/dev/null; then
        echo "$name: ${branch:-detached HEAD}, no upstream branch to compare with"
        continue
    fi
    ahead=$(git -C "$p" rev-list --count '@{u}..HEAD')
    behind=$(git -C "$p" rev-list --count 'HEAD..@{u}')
    dirty=$(git -C "$p" status --porcelain | grep -c .)
    say="$name ($branch):"
    if [ "$behind" -gt 0 ]; then
        if [ "${PULL_ON_RETURN:-no}" = "yes" ] && [ "$dirty" -eq 0 ] && [ "$ahead" -eq 0 ]; then
            git -C "$p" merge --ff-only --quiet '@{u}' </dev/null && say="$say pulled $(shimmer_offline_count "$behind" commit);" behind=0
        fi
        [ "$behind" -gt 0 ] && say="$say $(shimmer_offline_count "$behind" commit) to pull;"
    fi
    if [ "$ahead" -gt 0 ]; then
        if [ "${PUSH_ON_RETURN:-no}" = "yes" ] && [ "$behind" -eq 0 ]; then
            if git -C "$p" push --quiet </dev/null; then
                say="$say pushed $(shimmer_offline_count "$ahead" commit);"
            else
                say="$say $(shimmer_offline_count "$ahead" commit) to push (push failed, see the log);"
            fi
        else
            say="$say $(shimmer_offline_count "$ahead" commit) to push;"
        fi
    fi
    [ "$dirty" -gt 0 ] && say="$say $(shimmer_offline_count "$dirty" "uncommitted file");"
    case "$say" in *";") echo "${say%;}" ;; *) echo "$say up to date" ;; esac
done <<PROJECTS
$(shimmer_offline_existing)
PROJECTS
rm -f "$SHIMMER_RESULTS"
exit 0
