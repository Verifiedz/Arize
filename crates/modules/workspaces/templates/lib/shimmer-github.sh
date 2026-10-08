# Helpers for the github-inbox template (ADR 0025): one GitHub query through the gh CLI, read into
# lines the summary prints. Steps load it after shimmer-open.sh:
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-github.sh"
#
# Shimmer never holds a GitHub token: gh does, from `gh auth login` (GH_HOST for GitHub
# Enterprise). gh's own --jq does the JSON work, so jq isn't needed.
#
# POSIX sh only. This is your copy: Shimmer never changes it after the workspace is created.

SHIMMER_RESULTS="$SHIMMER_STATE_DIR/results"

# SCOPE ("owner/repo some-org") as search qualifiers: repo:owner/repo org:some-org.
shimmer_github_scope() {
    out=""
    for s in $SCOPE; do
        case "$s" in
            */*) out="$out repo:$s" ;;
            *) out="$out org:$s" ;;
        esac
    done
    echo "$out"
}

SHIMMER_GITHUB_QUERY='
query($reviews: String!, $mine: String!, $issues: String!) {
  reviews: search(query: $reviews, type: ISSUE, first: 30) {
    nodes { ... on PullRequest { number title url createdAt isDraft repository { nameWithOwner } author { login } } }
  }
  mine: search(query: $mine, type: ISSUE, first: 30) {
    nodes { ... on PullRequest { number title url isDraft reviewDecision repository { nameWithOwner }
      commits(last: 1) { nodes { commit { statusCheckRollup { state } } } } } }
  }
  issues: search(query: $issues, type: ISSUE, first: 30) {
    nodes { ... on Issue { number title url updatedAt repository { nameWithOwner } } }
  }
}'

# One line per item, tab-separated: section, repo#number, title, what to say about it, url, and
# for a review request how many days it has waited.
SHIMMER_GITHUB_JQ='
def days(t): ((now - (t | fromdateiso8601)) / 86400 | floor);
def ago(d): if d == 0 then "today" elif d == 1 then "1 day" else "\(d) days" end;
(.data.reviews.nodes[] | select(.number != null) | days(.createdAt) as $d
  | ["review", "\(.repository.nameWithOwner)#\(.number)", .title,
     "by \(.author.login // "ghost"), waiting \(ago($d))\(if .isDraft then ", draft" else "" end)", .url, ($d | tostring)] | @tsv),
(.data.mine.nodes[] | select(.number != null)
  | (.commits.nodes[0].commit.statusCheckRollup.state // "NONE") as $c
  | (.reviewDecision // "") as $r
  | (if $c == "SUCCESS" then "checks pass" elif $c == "FAILURE" or $c == "ERROR" then "checks failing"
     elif $c == "NONE" then "no checks" else "checks running" end) as $checks
  | (if $r == "APPROVED" then "approved" elif $r == "CHANGES_REQUESTED" then "changes requested"
     elif $r == "REVIEW_REQUIRED" then "waiting for review" else "" end) as $review
  | (if .isDraft then "draft"
     elif ($c == "SUCCESS" or $c == "NONE") and $r == "APPROVED" then "ready to merge"
     else ([$checks, $review] | map(select(. != "")) | join(", ")) end) as $state
  | ["mine", "\(.repository.nameWithOwner)#\(.number)", .title, $state, .url, ""] | @tsv),
(.data.issues.nodes[] | select(.number != null)
  | ["issue", "\(.repository.nameWithOwner)#\(.number)", .title, (days(.updatedAt) | if . == 0 then "updated today" else "updated \(ago(.)) ago" end), .url, ""] | @tsv)'

# Run the query; lines as above.
shimmer_github_fetch() {
    scope=$(shimmer_github_scope)
    gh api graphql \
        -f query="$SHIMMER_GITHUB_QUERY" \
        -f reviews="is:pr is:open archived:false review-requested:@me$scope" \
        -f mine="is:pr is:open archived:false author:@me$scope" \
        -f issues="is:issue is:open archived:false assignee:@me$scope" \
        --jq "$SHIMMER_GITHUB_JQ"
}

# Unread notifications (up to 50; "50+" past that).
shimmer_github_unread() {
    n=$(gh api 'notifications?per_page=50' --jq 'length' 2>/dev/null) || return 0
    if [ "$n" = "50" ]; then echo "50+"; else echo "$n"; fi
}

# The web address of the GitHub this gh is logged into.
shimmer_github_web() {
    echo "https://${GH_HOST:-github.com}"
}
