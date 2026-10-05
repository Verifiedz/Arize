# Supervised: print what Shimmer passes to every script (CLAUDE.md §10.1), and call back into
# the daemon through SHIMMER_SOCKET.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

echo "variables from Shimmer:"
env | grep '^SHIMMER_' | sort

for name in SHIMMER_WORKSPACE_ID SHIMMER_WORKSPACE_DIR SHIMMER_HOME SHIMMER_SOCKET SHIMMER_SESSION_ID SHIMMER_PLATFORM; do
    eval "value=\${$name:-}"
    [ -n "$value" ] || shimmer_fail "$name is not set"
done

if shimmer_has shimmer; then
    shimmer ping || shimmer_fail "shimmer ping failed through SHIMMER_SOCKET=$SHIMMER_SOCKET"
else
    echo "shimmer isn't on PATH here: skipped calling back into the daemon"
fi

[ "$FAIL_AT" = "check" ] && shimmer_fail "FAIL_AT = check: failing on purpose"
echo "check passed"
