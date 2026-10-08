# Supervised: wait until every local URL answers. If the apps stop, say so with the end of
# each app's log.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

[ -n "$LOCAL_URLS" ] || exit 0
shimmer_has curl || {
    echo "no curl: not waiting"
    exit 0
}
i=0
while [ $i -lt 170 ]; do
    pending=""
    for url in $LOCAL_URLS; do
        shimmer_answers "$url" || pending="$pending $url"
    done
    if [ -z "$pending" ]; then
        echo "all up:$(printf ' %s' $LOCAL_URLS)"
        exit 0
    fi
    # A dev server never exits on its own: one that has means that app failed to start.
    for pidfile in "$SHIMMER_STATE_DIR"/dev-*.pid; do
        [ -f "$pidfile" ] || continue
        kill -0 "$(cat "$pidfile")" 2>/dev/null && continue
        app=$(basename "$pidfile" .pid | sed 's/^dev-//')
        echo "the $app app stopped. The end of its log:" >&2
        tail -n 15 "$SHIMMER_STATE_DIR/dev-$app.log" >&2
        exit 1
    done
    sleep 1
    i=$((i + 1))
done
shimmer_fail "still not answering after 170 seconds:$pending (logs: $SHIMMER_STATE_DIR/dev-*.log)"
