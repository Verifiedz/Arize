# Supervised: stop with one clear sentence before anything starts.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-monorepo.sh"

problem=$(shimmer_display_problem)
[ -z "$problem" ] || shimmer_fail "$problem"

[ -n "$PROJECT_DIR" ] || shimmer_fail "PROJECT_DIR is empty: set it with shimmer workspaces edit $SHIMMER_WORKSPACE_ID"
[ -d "$PROJECT_DIR" ] || shimmer_fail "the monorepo folder $PROJECT_DIR doesn't exist"
[ -n "$APPS" ] || shimmer_fail "APPS is empty: list the apps to run"

tool=$(shimmer_mono_tool "$PROJECT_DIR")
if [ "${DEV_COMMAND:-auto}" = "auto" ]; then
    [ -n "$tool" ] || shimmer_fail "no monorepo tool found in $PROJECT_DIR (turbo.json, nx.json, pnpm-workspace.yaml, package.json workspaces, Cargo.toml [workspace], go.work): set DEV_COMMAND"
    program=$(shimmer_mono_program "$tool" "$PROJECT_DIR")
    shimmer_has "$program" || shimmer_fail "this monorepo uses $tool, which needs $program: it isn't installed"
fi

shimmer_has_editor "${CODE_EDITOR:-none}" || shimmer_fail "${CODE_EDITOR} isn't installed"
shimmer_has_terminal "${TERMINAL_APP:-none}" || shimmer_fail "the terminal '$TERMINAL_APP' isn't installed here"
if [ "$CODE_EDITOR" = "neovim" ] && [ "${TERMINAL_APP:-none}" = "none" ]; then
    shimmer_fail "neovim opens in a terminal: set TERMINAL_APP to auto or a terminal"
fi
[ "$GIT_ON_OPEN" = "pull" ] && ! shimmer_has git && shimmer_fail "git isn't installed (or set GIT_ON_OPEN to none)"

if [ -n "$LOCAL_URLS" ] && shimmer_has curl && [ -z "$(shimmer_dev_server_pid)" ]; then
    for url in $LOCAL_URLS; do
        shimmer_answers "$url" && shimmer_fail "something else is already running at $url: stop it, or change LOCAL_URLS"
    done
fi
echo "ok: $PROJECT_DIR (${tool:-custom}), apps: $APPS"
