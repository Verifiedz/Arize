# Supervised: stop with one clear sentence before anything opens, instead of halfway.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"

problem=$(shimmer_display_problem)
[ -z "$problem" ] || shimmer_fail "$problem"

[ -n "$PROJECT_DIR" ] || shimmer_fail "PROJECT_DIR is empty: set it in workspace.toml's [env]"
[ -d "$PROJECT_DIR" ] || shimmer_fail "the project folder $PROJECT_DIR doesn't exist"

shimmer_has_editor "${CODE_EDITOR:-none}" || shimmer_fail "${CODE_EDITOR} isn't installed (looked for $(shimmer_editor_commands "$CODE_EDITOR") and the app)"
shimmer_has_terminal "${TERMINAL_APP:-none}" || shimmer_fail "the terminal '$TERMINAL_APP' isn't installed here"
if [ "$CODE_EDITOR" = "neovim" ] && [ "${TERMINAL_APP:-none}" = "none" ]; then
    shimmer_fail "neovim opens in a terminal: set TERMINAL_APP to auto or a terminal"
fi

pm=$(shimmer_package_manager "$PROJECT_DIR")
dev=$(shimmer_dev_command "$PROJECT_DIR" "$DEV_COMMAND")
if [ -n "$pm" ] && { [ "$INSTALL_COMMAND" = "auto" ] || [ "$DEV_COMMAND" = "auto" ]; }; then
    shimmer_has "$pm" || shimmer_fail "this project uses $pm, which isn't installed"
fi
[ "$GIT_ON_OPEN" = "pull" ] && ! shimmer_has git && shimmer_fail "git isn't installed (or set GIT_ON_OPEN to none)"

url=$(shimmer_local_url "$PROJECT_DIR" "$LOCAL_URL")
if [ -n "$url" ] && [ -n "$dev" ] && shimmer_has curl && [ -z "$(shimmer_dev_server_pid)" ]; then
    shimmer_answers "$url" && shimmer_fail "something else is already running at $url: stop it, or change LOCAL_URL"
fi
echo "ok: $PROJECT_DIR${pm:+ ($pm)}${dev:+, dev: $dev}${url:+, $url}"
