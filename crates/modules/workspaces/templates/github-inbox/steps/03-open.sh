# Detached: open what OPEN says in the browser, at most 10 tabs.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-github.sh"

web=$(shimmer_github_web)
case "${OPEN:-nothing}" in
    nothing) exit 0 ;;
    inbox-pages) shimmer_open_url "$web/pulls/review-requested" "$web/notifications" ;;
    reviews) shimmer_open_url $(grep '^review	' "$SHIMMER_RESULTS" | cut -f 5 | head -n 10) ;;
    everything) shimmer_open_url $(cut -f 5 "$SHIMMER_RESULTS" | head -n 10) ;;
esac
