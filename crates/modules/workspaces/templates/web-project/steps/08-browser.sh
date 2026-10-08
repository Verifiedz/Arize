# Detached: localhost, your live site, the repository page and your links, in your normal browser or a window of
# the workspace's own (BROWSER_WINDOW) that stop can close.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

local_url=""
[ -n "$(shimmer_dev_command "$PROJECT_DIR" "$DEV_COMMAND")" ] && local_url=$(shimmer_local_url "$PROJECT_DIR" "$LOCAL_URL")
live_url=$(shimmer_live_url "$PROJECT_DIR" "${LIVE_URL:-}")
repo=$(shimmer_repo_url "$PROJECT_DIR" "${REPO_PAGE:-none}")
# LINKS is space-separated on purpose: split it into words.
# shellcheck disable=SC2086
set -- $local_url $live_url $repo $LINKS
[ $# -gt 0 ] || exit 0
if [ "${BROWSER_WINDOW:-shared}" = "separate" ] && [ -z "$BROWSER" ]; then
    shimmer_open_urls_window "${BROWSER_APP:-auto}" "$@"
else
    [ "${BROWSER_WINDOW:-shared}" = "separate" ] && echo "a separate browser window can't open on the computer you're connected from: using your normal browser there"
    shimmer_open_url "$@"
    shimmer_record_left_open "the tabs in your normal browser (set BROWSER_WINDOW = separate to have stop close them)"
fi
