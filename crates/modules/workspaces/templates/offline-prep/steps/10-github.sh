# Supervised: your GitHub issues and PRs as Markdown, to read and review offline: issues
# assigned to you, your open PRs, and PRs waiting for your review, each with its comments, and
# PRs with their diff. Through gh, so Shimmer never holds a GitHub token.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

shimmer_offline_skipped github && exit 0
[ "${GITHUB_SNAPSHOT:-projects}" = "no" ] && exit 0
if ! shimmer_has gh || ! gh auth status >/dev/null 2>&1; then
    shimmer_offline_result warn github "no GitHub snapshot: gh isn't installed or logged in (gh auth login)"
    exit 0
fi

# Which repos: these projects' GitHub repos, or all of yours.
scope=""
if [ "${GITHUB_SNAPSHOT:-projects}" = "projects" ]; then
    while IFS= read -r p; do
        [ -n "$p" ] || continue
        repo=$(shimmer_offline_github_repo "$p")
        [ -n "$repo" ] && scope="$scope repo:$repo"
    done <<PROJECTS
$(shimmer_offline_existing)
PROJECTS
    if [ -z "$scope" ]; then
        shimmer_offline_result info github "none of the projects is on GitHub, so no snapshot"
        exit 0
    fi
fi

dir="$(shimmer_offline_dir)/github"
rm -rf "$dir"
mkdir -p "$dir"
tab=$(printf '\t')
items=$(gh api graphql \
    -f query='query($a: String!, $b: String!, $c: String!) {
      a: search(query: $a, type: ISSUE, first: 30) { nodes { ... on Issue { number repository { nameWithOwner } } } }
      b: search(query: $b, type: ISSUE, first: 30) { nodes { ... on PullRequest { number repository { nameWithOwner } } } }
      c: search(query: $c, type: ISSUE, first: 30) { nodes { ... on PullRequest { number repository { nameWithOwner } } } } }' \
    -f a="is:issue is:open assignee:@me archived:false$scope" \
    -f b="is:pr is:open author:@me archived:false$scope" \
    -f c="is:pr is:open review-requested:@me archived:false$scope" \
    --jq '(.data.a.nodes[] | select(.number) | ["issue", .repository.nameWithOwner, .number] | @tsv),
          (.data.b.nodes[], .data.c.nodes[] | select(.number) | ["pr", .repository.nameWithOwner, .number] | @tsv)' </dev/null) || {
    shimmer_offline_result warn github "gh couldn't ask GitHub (see the github step's log)"
    exit 0
}
issues=0
prs=0
while IFS="$tab" read -r kind repo number; do
    [ -n "$number" ] || continue
    base="$dir/$(printf '%s' "$repo" | tr '/' '-')-$number"
    if [ "$kind" = issue ]; then
        gh issue view "$number" -R "$repo" --comments </dev/null >"$base.md" 2>/dev/null && issues=$((issues + 1))
    else
        gh pr view "$number" -R "$repo" --comments </dev/null >"$base.md" 2>/dev/null && prs=$((prs + 1))
        gh pr diff "$number" -R "$repo" </dev/null >"$base.diff" 2>/dev/null
    fi
done <<ITEMS
$items
ITEMS
shimmer_offline_result ok github "saved $issues issues and $prs PRs (with diffs) to $dir"
exit 0
