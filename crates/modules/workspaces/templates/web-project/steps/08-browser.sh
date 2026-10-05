# Detached: localhost, the repository page and your links.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

local_url=""
[ -n "$(shimmer_dev_command "$PROJECT_DIR" "$DEV_COMMAND")" ] && local_url=$(shimmer_local_url "$PROJECT_DIR" "$LOCAL_URL")
repo=$(shimmer_repo_url "$PROJECT_DIR" "${REPO_PAGE:-none}")
# LINKS is space-separated on purpose: split it into words.
# shellcheck disable=SC2086
shimmer_open_url "$local_url" "$repo" $LINKS
