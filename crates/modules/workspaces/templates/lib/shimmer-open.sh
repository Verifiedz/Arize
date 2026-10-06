# Shared helpers for workspace templates (ADR 0025 §5). Steps load it with
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
#
# Everything that differs between Linux and macOS lives here, so step scripts stay short.
# POSIX sh only: on Debian and Ubuntu `sh` is dash (ADR 0012 §5).
#
# This is your copy: Shimmer never changes it after the workspace is created.

# Where this workspace keeps its dev server's log and process id: the system temp folder, never
# inside the Shimmer folder (only the daemon writes there). A reboot clears it, and a reboot
# also ends the server.
SHIMMER_STATE_DIR="${TMPDIR:-/tmp}/shimmer-$SHIMMER_WORKSPACE_ID"

shimmer_has() {
    command -v "$1" >/dev/null 2>&1
}

shimmer_macos() {
    [ "$SHIMMER_PLATFORM" = "macos" ]
}

# Print an error for the step's log and stop the step. In a supervised step this marks the
# workspace dirty with this message.
shimmer_fail() {
    echo "$*" >&2
    exit 1
}

# ---------------------------------------------------------------- where windows open

# Where editors, browsers and terminals appear (OPEN_ON). Steps inherit the daemon's settings,
# which come from whatever terminal last started it: a Cursor or VS Code terminal connected from
# another computer sends windows to that computer, the machine's own desktop keeps them here. So
# a workspace can say where it wants them instead:
#   auto                as the daemon was started (unchanged)
#   this-machine        this computer's own screen, ignoring any editor's remote-connection helpers
#   connected-computer  the computer you're connected from, through Cursor's or VS Code's helpers
shimmer_use_display() {
    case "${OPEN_ON:-auto}" in
        this-machine)
            unset BROWSER VSCODE_IPC_HOOK_CLI
            PATH=$(printf '%s' "$PATH" | tr ':' '\n' | grep -v -e '/\.cursor-server/' -e '/\.vscode-server/' | paste -s -d: -)
            export PATH
            shimmer_macos && return 0
            XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
            export XDG_RUNTIME_DIR
            if [ -z "$WAYLAND_DISPLAY" ]; then
                for sock in "$XDG_RUNTIME_DIR"/wayland-*; do
                    case "$sock" in *.lock) continue ;; esac
                    if [ -S "$sock" ]; then
                        WAYLAND_DISPLAY=$(basename "$sock")
                        export WAYLAND_DISPLAY
                        break
                    fi
                done
            fi
            if [ -z "$DISPLAY" ] && [ -S /tmp/.X11-unix/X0 ]; then
                DISPLAY=:0
                export DISPLAY
            fi
            if [ -z "$DBUS_SESSION_BUS_ADDRESS" ] && [ -S "$XDG_RUNTIME_DIR/bus" ]; then
                DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
                export DBUS_SESSION_BUS_ADDRESS
            fi
            ;;
        connected-computer)
            bin=$(ls -td "$HOME"/.cursor-server/bin/*/*/bin "$HOME"/.cursor-server/bin/*/bin \
                "$HOME"/.vscode-server/bin/*/bin "$HOME"/.vscode-server/cli/servers/*/server/bin 2>/dev/null | head -n 1)
            [ -n "$bin" ] || return 0
            [ -d "$bin/remote-cli" ] && PATH="$bin/remote-cli:$PATH" && export PATH
            [ -f "$bin/helpers/browser.sh" ] && BROWSER="$bin/helpers/browser.sh" && export BROWSER
            if [ -z "$VSCODE_IPC_HOOK_CLI" ]; then
                rt=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
                VSCODE_IPC_HOOK_CLI=$(ls -t "$rt"/vscode-ipc-*.sock 2>/dev/null | head -n 1)
                [ -n "$VSCODE_IPC_HOOK_CLI" ] && export VSCODE_IPC_HOOK_CLI
            fi
            ;;
    esac
}

# Why windows can't open where OPEN_ON asks, or nothing. For the check step.
shimmer_display_problem() {
    case "${OPEN_ON:-auto}" in
        this-machine)
            shimmer_macos && return 0
            [ -n "$WAYLAND_DISPLAY" ] || [ -n "$DISPLAY" ] ||
                echo "OPEN_ON is this-machine, but this computer has no desktop session open: log in on its screen first"
            ;;
        connected-computer)
            [ -n "$VSCODE_IPC_HOOK_CLI" ] && [ -S "$VSCODE_IPC_HOOK_CLI" ] ||
                echo "OPEN_ON is connected-computer, but no Cursor or VS Code window is connected to this computer right now"
            ;;
    esac
}

# ---------------------------------------------------------------- urls

# The default browser. When BROWSER is set it wins: over a remote session (Cursor or VS Code
# connected to this machine) it's the helper that opens the link on the computer you're sitting at.
shimmer_open_url() {
    for url in "$@"; do
        [ -n "$url" ] || continue
        if [ -n "$BROWSER" ]; then
            "$BROWSER" "$url" >/dev/null 2>&1 &
        elif shimmer_macos; then
            open "$url"
        elif shimmer_has xdg-open; then
            xdg-open "$url" >/dev/null 2>&1 &
        else
            echo "no xdg-open: can't open $url" >&2
        fi
    done
}

# ---------------------------------------------------------------- editors

# The shell commands an editor may be installed as on Linux, best first.
shimmer_editor_commands() {
    case "$1" in
        vscode) echo "code codium" ;;
        cursor) echo "cursor" ;;
        zed) echo "zed zeditor zedit" ;;
        webstorm) echo "webstorm" ;;
        intellij) echo "idea idea-ultimate idea-community intellij-idea-ultimate intellij-idea-community" ;;
        sublime) echo "subl" ;;
        neovim) echo "nvim" ;;
        *) echo "" ;;
    esac
}

# The app name on macOS.
shimmer_editor_app() {
    case "$1" in
        vscode) echo "Visual Studio Code" ;;
        cursor) echo "Cursor" ;;
        zed) echo "Zed" ;;
        webstorm) echo "WebStorm" ;;
        intellij) echo "IntelliJ IDEA" ;;
        sublime) echo "Sublime Text" ;;
        *) echo "" ;;
    esac
}

# The first installed command for EDITOR, or nothing.
shimmer_editor_command() {
    for c in $(shimmer_editor_commands "$1"); do
        if shimmer_has "$c"; then
            echo "$c"
            return
        fi
    done
}

# Succeeds when EDITOR can be opened here.
shimmer_has_editor() {
    [ "$1" = "none" ] && return 0
    [ -n "$(shimmer_editor_command "$1")" ] && return 0
    if shimmer_macos; then
        app=$(shimmer_editor_app "$1")
        [ -n "$app" ] && open -Ra "$app" >/dev/null 2>&1 && return 0
        # IntelliJ's free edition has its own app name.
        [ "$1" = "intellij" ] && open -Ra "IntelliJ IDEA CE" >/dev/null 2>&1 && return 0
    fi
    return 1
}

# Open EDITOR on PATH (a folder or a file). Neovim runs in a terminal (TERMINAL_APP).
shimmer_open_editor() {
    editor=$1
    target=$2
    case "$editor" in
        none) return 0 ;;
        neovim)
            dir=$target
            [ -d "$dir" ] || dir=$(dirname "$target")
            shimmer_open_terminal "${TERMINAL_APP:-auto}" "$dir" "nvim $(shimmer_quote "$target")"
            return
            ;;
    esac
    cmd=$(shimmer_editor_command "$editor")
    if [ -n "$cmd" ]; then
        "$cmd" "$target" >/dev/null 2>&1 &
        return 0
    fi
    if shimmer_macos; then
        app=$(shimmer_editor_app "$editor")
        case "$editor" in
            # JetBrains apps take the project as an argument to a new instance.
            webstorm | intellij) open -na "$app" --args "$target" ;;
            *) open -a "$app" "$target" ;;
        esac
        return
    fi
    shimmer_fail "$editor isn't installed"
}

# ---------------------------------------------------------------- a terminal inside the editor

# A string as a JSON string literal.
shimmer_json_string() {
    printf '"%s"' "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')"
}

# Make VS Code / Cursor open a terminal running COMMAND inside the editor whenever FOLDER opens:
# a task in FOLDER/.vscode/tasks.json with `"runOn": "folderOpen"`, the editors' own feature
# for this. Only ever writes or removes a tasks.json that Shimmer created (it says so on its
# first line); a tasks.json of yours is left alone and the task to add is printed instead.
# Kept out of git through .git/info/exclude, which is local and never committed. An empty
# COMMAND removes Shimmer's file.
shimmer_ide_terminal() {
    folder=$1
    command=$2
    file="$folder/.vscode/tasks.json"
    marker="// Created by Shimmer workspace '$SHIMMER_WORKSPACE_ID'"
    ours=false
    [ -f "$file" ] && head -n 1 "$file" | grep -qF "// Created by Shimmer workspace" && ours=true
    if [ -z "$command" ]; then
        if $ours; then
            rm -f "$file"
            echo "removed Shimmer's $file"
        fi
        return 0
    fi
    task=$(cat <<TASK
    {
      "label": $(shimmer_json_string "Shimmer: $command"),
      "type": "shell",
      "command": $(shimmer_json_string "$command"),
      "isBackground": true,
      "problemMatcher": [],
      "presentation": { "reveal": "always", "panel": "dedicated", "focus": false },
      "runOptions": { "runOn": "folderOpen" }
    }
TASK
)
    if [ -f "$file" ] && ! $ours; then
        echo "$file is yours, so Shimmer won't change it. To get the terminal, add this to its \"tasks\":"
        echo "$task"
        return 0
    fi
    mkdir -p "$folder/.vscode" || return 0
    {
        echo "$marker: opens a terminal in the editor"
        echo "// running your command when this folder opens. Change it with: shimmer workspaces edit $SHIMMER_WORKSPACE_ID"
        echo "// (IDE_TERMINAL_COMMAND). Shimmer rewrites this file on every launch; it's kept out of git."
        echo "{"
        echo '  "version": "2.0.0",'
        echo '  "tasks": ['
        echo "$task"
        echo "  ]"
        echo "}"
    } >"$file"
    # Hidden from `git status` on this machine only, unless the file is already tracked.
    if git -C "$folder" rev-parse --git-dir >/dev/null 2>&1 && ! git -C "$folder" ls-files --error-unmatch .vscode/tasks.json >/dev/null 2>&1; then
        exclude="$(git -C "$folder" rev-parse --git-path info/exclude)"
        case "$exclude" in /*) ;; *) exclude="$folder/$exclude" ;; esac
        mkdir -p "$(dirname "$exclude")"
        grep -qxF "/.vscode/tasks.json" "$exclude" 2>/dev/null || echo "/.vscode/tasks.json" >>"$exclude"
    fi
    echo "the editor will open a terminal running: $command"
}

# ---------------------------------------------------------------- terminals

# 'it'\''s' style quoting, so a path or command survives being passed through another shell.
shimmer_quote() {
    printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

# The terminal TERMINAL_APP=auto means: Terminal on macOS, the first one found on Linux.
shimmer_terminal_auto() {
    if shimmer_macos; then
        echo "terminal"
        return
    fi
    for t in kitty foot alacritty wezterm ghostty gnome-terminal konsole xterm; do
        if shimmer_has "$t"; then
            echo "$t"
            return
        fi
    done
}

# Succeeds when TERMINAL can be opened here.
shimmer_has_terminal() {
    case "$1" in
        none) return 0 ;;
        auto) [ -n "$(shimmer_terminal_auto)" ] ;;
        terminal) shimmer_macos ;;
        iterm) shimmer_macos && open -Ra "iTerm" >/dev/null 2>&1 ;;
        *) shimmer_has "$1" ;;
    esac
}

# Open TERMINAL in DIR, running COMMAND (a shell command line) if given. The terminal stays open
# afterwards, at a shell in DIR.
shimmer_open_terminal() {
    term=$1
    dir=$2
    command=$3
    [ "$term" = "auto" ] && term=$(shimmer_terminal_auto)
    [ -n "$term" ] || shimmer_fail "no terminal found (set TERMINAL_APP in workspace.toml)"
    [ "$term" = "none" ] && return 0
    shell=${SHELL:-sh}
    # What runs inside the terminal: the command, then an interactive shell.
    inner="cd $(shimmer_quote "$dir") && $command; exec $(shimmer_quote "$shell")"
    case "$term" in
        terminal | iterm)
            if [ -z "$command" ]; then
                if [ "$term" = "terminal" ]; then open -a Terminal "$dir"; else open -a iTerm "$dir"; fi
                return
            fi
            line=$(printf '%s' "cd $(shimmer_quote "$dir") && $command" | sed 's/\\/\\\\/g; s/"/\\"/g')
            if [ "$term" = "terminal" ]; then
                osascript -e "tell application \"Terminal\" to do script \"$line\"" -e 'tell application "Terminal" to activate' >/dev/null
            else
                osascript -e 'tell application "iTerm"' -e 'set w to (create window with default profile)' \
                    -e "tell current session of w to write text \"$line\"" -e 'end tell' >/dev/null
            fi
            return
            ;;
    esac
    if [ -z "$command" ]; then
        case "$term" in
            kitty) kitty --directory "$dir" >/dev/null 2>&1 & ;;
            foot) foot --working-directory="$dir" >/dev/null 2>&1 & ;;
            alacritty) alacritty --working-directory "$dir" >/dev/null 2>&1 & ;;
            wezterm) wezterm start --cwd "$dir" >/dev/null 2>&1 & ;;
            ghostty) ghostty --working-directory="$dir" >/dev/null 2>&1 & ;;
            gnome-terminal) gnome-terminal --working-directory="$dir" >/dev/null 2>&1 & ;;
            konsole) konsole --workdir "$dir" >/dev/null 2>&1 & ;;
            *) (cd "$dir" && "$term" >/dev/null 2>&1 &) ;;
        esac
        return 0
    fi
    case "$term" in
        kitty) kitty --directory "$dir" "$shell" -c "$inner" >/dev/null 2>&1 & ;;
        foot) foot --working-directory="$dir" "$shell" -c "$inner" >/dev/null 2>&1 & ;;
        wezterm) wezterm start --cwd "$dir" -- "$shell" -c "$inner" >/dev/null 2>&1 & ;;
        gnome-terminal) gnome-terminal --working-directory="$dir" -- "$shell" -c "$inner" >/dev/null 2>&1 & ;;
        konsole) konsole --workdir "$dir" -e "$shell" -c "$inner" >/dev/null 2>&1 & ;;
        *) "$term" -e "$shell" -c "$inner" >/dev/null 2>&1 & ;;
    esac
    return 0
}

# ---------------------------------------------------------------- web projects

# The package manager a project uses, from its lockfile; nothing without a package.json.
shimmer_package_manager() {
    dir=$1
    [ -f "$dir/package.json" ] || return 0
    if [ -f "$dir/pnpm-lock.yaml" ]; then
        echo pnpm
    elif [ -f "$dir/yarn.lock" ]; then
        echo yarn
    elif [ -f "$dir/bun.lockb" ] || [ -f "$dir/bun.lock" ]; then
        echo bun
    else
        echo npm
    fi
}

shimmer_lockfile() {
    case "$1" in
        pnpm) echo pnpm-lock.yaml ;;
        yarn) echo yarn.lock ;;
        bun) if [ -f "$2/bun.lock" ]; then echo bun.lock; else echo bun.lockb; fi ;;
        npm) echo package-lock.json ;;
    esac
}

# Does package.json have this script?
shimmer_has_script() {
    grep -q "\"$2\"[[:space:]]*:" "$1/package.json" 2>/dev/null
}

# Does package.json name this package (as a dependency)?
shimmer_has_package() {
    grep -q "\"$2\"[[:space:]]*:" "$1/package.json" 2>/dev/null
}

# DEV_COMMAND, with `auto` worked out: `<pm> run dev`, else `<pm> start`, else nothing.
shimmer_dev_command() {
    dir=$1
    answer=$2
    [ "$answer" = "auto" ] || {
        echo "$answer"
        return
    }
    pm=$(shimmer_package_manager "$dir")
    [ -n "$pm" ] || return 0
    if shimmer_has_script "$dir" dev; then
        echo "$pm run dev"
    elif shimmer_has_script "$dir" start; then
        echo "$pm start"
    fi
}

# LOCAL_URL, with `auto` worked out from the framework the project uses.
shimmer_local_url() {
    dir=$1
    answer=$2
    [ "$answer" = "auto" ] || {
        echo "$answer"
        return
    }
    for pair in next:3000 nuxt:3000 react-scripts:3000 astro:4321 gatsby:8000 @sveltejs/kit:5173 vite:5173; do
        if shimmer_has_package "$dir" "${pair%%:*}"; then
            echo "http://localhost:${pair##*:}"
            return
        fi
    done
}

# Run a command line in DIR with the Node version the project asks for (.nvmrc or
# .node-version), when fnm or nvm is installed. Otherwise as it is.
shimmer_run_in_project() {
    dir=$1
    command=$2
    cd "$dir" || shimmer_fail "can't enter $dir"
    if [ -f .nvmrc ] || [ -f .node-version ]; then
        if shimmer_has fnm; then
            eval "$(fnm env)" && fnm use --install-if-missing >/dev/null 2>&1
        elif [ -s "${NVM_DIR:-$HOME/.nvm}/nvm.sh" ]; then
            . "${NVM_DIR:-$HOME/.nvm}/nvm.sh" && nvm use >/dev/null 2>&1
        fi
    fi
    sh -c "$command"
}

# The project's repository page, from `git remote get-url origin`, as https. PAGE is home,
# pulls, issues or actions; the last three only on GitHub.
shimmer_repo_url() {
    dir=$1
    page=$2
    [ "$page" = "none" ] && return 0
    remote=$(git -C "$dir" remote get-url origin 2>/dev/null) || return 0
    case "$remote" in
        git@*:*) url="https://$(printf '%s' "${remote#git@}" | sed 's/:/\//')" ;;
        ssh://*) url="https://$(printf '%s' "${remote#ssh://}" | sed 's/^[^@]*@//; s/:[0-9]*\//\//')" ;;
        https://* | http://*) url=$(printf '%s' "$remote" | sed 's#//[^@/]*@#//#') ;;
        *) return 0 ;;
    esac
    url=${url%.git}
    case "$page:$url" in
        home:*) echo "$url" ;;
        pulls:https://github.com/* | issues:https://github.com/* | actions:https://github.com/*) echo "$url/$page" ;;
        *) echo "$url" ;;
    esac
}

# Does something answer at URL?
shimmer_answers() {
    curl -s -o /dev/null --max-time 2 "$1"
}

# The process id of this workspace's dev server, if it's still running.
shimmer_dev_server_pid() {
    file="$SHIMMER_STATE_DIR/dev-server.pid"
    [ -f "$file" ] || return 0
    pid=$(cat "$file")
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null && echo "$pid"
}

# Stop a process and everything it started: a detached step is its own process group.
shimmer_stop_group() {
    pid=$1
    kill -s TERM -- "-$pid" 2>/dev/null || kill -s TERM "$pid" 2>/dev/null
    i=0
    while kill -0 "$pid" 2>/dev/null && [ $i -lt 10 ]; do
        sleep 0.5
        i=$((i + 1))
    done
    kill -s KILL -- "-$pid" 2>/dev/null
    return 0
}


# Every step that loads this file opens things where OPEN_ON says.
shimmer_use_display
