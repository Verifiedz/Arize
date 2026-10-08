# Supervised: pull only when it's safe, and never stop the launch over it (you may be offline).
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

[ "$GIT_ON_OPEN" = "pull" ] || exit 0
cd "$PROJECT_DIR" || exit 0
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || {
    echo "not a git repository: nothing to pull"
    exit 0
}
if [ -n "$(git status --porcelain)" ]; then
    echo "you have uncommitted changes: not pulling"
    exit 0
fi
git rev-parse --abbrev-ref '@{u}' >/dev/null 2>&1 || {
    echo "this branch has no upstream: nothing to pull"
    exit 0
}
# Never wait on a password prompt nobody can see.
GIT_TERMINAL_PROMPT=0 git pull --ff-only || echo "couldn't pull (offline, or the branch has diverged): continuing with what you have"
exit 0
