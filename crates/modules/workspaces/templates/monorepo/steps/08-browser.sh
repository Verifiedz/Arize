# Detached: every app's local URL, your live site, the repository page and your links, in your
# normal browser or a window of the workspace's own (BROWSER_WINDOW) that stop can close.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

live_url=$(shimmer_live_url "$PROJECT_DIR" "${LIVE_URL:-}")
repo=$(shimmer_repo_url "$PROJECT_DIR" "${REPO_PAGE:-none}")
# LOCAL_URLS and LINKS are space-separated on purpose: split them into words.
# shellcheck disable=SC2086
set -- $LOCAL_URLS $live_url $repo $LINKS
[ $# -gt 0 ] || exit 0
if [ "${BROWSER_WINDOW:-shared}" = "separate" ] && [ -z "$BROWSER" ]; then
    shimmer_open_urls_window "${BROWSER_APP:-auto}" "$@"
else
    [ "${BROWSER_WINDOW:-shared}" = "separate" ] && echo "a separate browser window can't open on the computer you're connected from: using your normal browser there"
    shimmer_open_url "$@"
    shimmer_record_left_open "the tabs in your normal browser (set BROWSER_WINDOW = separate to have stop close them)"
fi
