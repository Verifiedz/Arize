# Supervised: one GitHub query for all three lists, and the unread count. A failure here (no
# network, GitHub down) makes the workspace dirty with gh's own message, rather than an inbox
# that looks empty.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-github.sh"

mkdir -p "$SHIMMER_STATE_DIR"
shimmer_github_fetch >"$SHIMMER_RESULTS.new" || shimmer_fail "gh couldn't ask GitHub (its message is above)"
mv "$SHIMMER_RESULTS.new" "$SHIMMER_RESULTS"
shimmer_github_unread >"$SHIMMER_STATE_DIR/unread"
echo "fetched $(grep -c . "$SHIMMER_RESULTS") items"
