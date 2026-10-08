# Supervised: wait until the local URL answers, so the browser never shows "can't connect".
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

url=$(shimmer_local_url "$PROJECT_DIR" "$LOCAL_URL")
dev=$(shimmer_dev_command "$PROJECT_DIR" "$DEV_COMMAND")
[ -n "$url" ] && [ -n "$dev" ] || exit 0
shimmer_has curl || {
    echo "no curl: not waiting for $url"
    exit 0
}
i=0
while [ $i -lt 100 ]; do
    shimmer_answers "$url" && {
        echo "$url is up"
        exit 0
    }
    if [ $i -gt 2 ] && [ -z "$(shimmer_dev_server_pid)" ]; then
        echo "the dev server stopped. The end of its log:" >&2
        tail -n 20 "$SHIMMER_STATE_DIR/dev-server.log" >&2 2>/dev/null
        exit 1
    fi
    sleep 1
    i=$((i + 1))
done
shimmer_fail "$url didn't answer within 100 seconds (the dev server's log: $SHIMMER_STATE_DIR/dev-server.log)"
