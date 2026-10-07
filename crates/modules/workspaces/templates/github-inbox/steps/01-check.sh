# Supervised: stop with one clear sentence before asking GitHub anything.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-github.sh"

shimmer_has gh || shimmer_fail "the gh CLI isn't installed: https://cli.github.com (then: gh auth login)"
gh auth status >/dev/null 2>&1 || shimmer_fail "gh isn't logged in${GH_HOST:+ to $GH_HOST}: run gh auth login"
case "${STALE_DAYS:-2}" in *[!0-9]*) shimmer_fail "STALE_DAYS must be a whole number, not '$STALE_DAYS'" ;; esac
case "${OPEN:-nothing}" in nothing | reviews | everything | inbox-pages) ;; *) shimmer_fail "OPEN must be nothing, reviews, everything or inbox-pages" ;; esac
if [ "${OPEN:-nothing}" != "nothing" ]; then
    problem=$(shimmer_display_problem)
    [ -z "$problem" ] || shimmer_fail "$problem"
fi
mkdir -p "$SHIMMER_STATE_DIR"
rm -f "$SHIMMER_RESULTS" "$SHIMMER_STATE_DIR/unread"
echo "ok: gh is logged in${SCOPE:+, only $SCOPE}"
