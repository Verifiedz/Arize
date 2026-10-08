# Helpers for the update-everything template (ADR 0025): which updaters are installed, how to
# run each one, and the record of what happened for the summary. Steps load it after
# shimmer-open.sh:
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-update.sh"
#
# POSIX sh only. This is your copy: Shimmer never changes it after the workspace is created, so
# add your own updaters to the list below if you like (or use EXTRA_COMMANDS).

SHIMMER_RESULTS="$SHIMMER_STATE_DIR/results"

# Is NAME in SKIP (names separated by spaces)?
shimmer_update_skipped() {
    case " $SKIP " in *" $1 "*) return 0 ;; esac
    return 1
}

# One line for the summary: "updated rustup", "failed npm (exit 1)", "skipped …".
shimmer_update_result() {
    mkdir -p "$SHIMMER_STATE_DIR"
    echo "$*" >>"$SHIMMER_RESULTS"
}

# MODE = preview: nothing runs; the summary lists what update would run.
shimmer_update_preview() {
    [ "${MODE:-preview}" != "update" ]
}

# Run one updater, record how it went, and carry on: one failure never stops the others.
# Input is /dev/null, so nothing can sit waiting for an answer nobody will type. In preview, only
# record what would run.
shimmer_update_run() {
    name=$1
    cmd=$2
    if shimmer_update_preview; then
        shimmer_update_result "would $name: $cmd"
        return 0
    fi
    echo
    echo "== $name: $cmd"
    sh -c "$cmd" </dev/null
    code=$?
    if [ "$code" -eq 0 ]; then
        shimmer_update_result "updated $name"
    else
        shimmer_update_result "failed $name (exit $code)"
    fi
}

# Use topgrade instead of the list below (USE_TOPGRADE)?
shimmer_update_use_topgrade() {
    case "${USE_TOPGRADE:-auto}" in
        yes) return 0 ;;
        no) return 1 ;;
        *) shimmer_has topgrade ;;
    esac
}

# Every installed updater this template knows, as "name<TAB>command" lines. Your own system's
# packages (pacman, apt…) are not here: they need sudo, so they're the system step's job.
shimmer_update_tools() {
    tab=$(printf '\t')
    shimmer_has rustup && echo "rustup${tab}rustup update"
    shimmer_has cargo-install-update && echo "cargo${tab}cargo install-update -a"
    shimmer_has brew && echo "brew${tab}brew update && brew upgrade"
    shimmer_has mas && echo "mas${tab}mas upgrade"
    shimmer_has npm && echo "npm${tab}npm update -g"
    shimmer_has pnpm && echo "pnpm${tab}pnpm update -g"
    shimmer_has bun && echo "bun${tab}bun upgrade"
    shimmer_has deno && echo "deno${tab}deno upgrade"
    shimmer_has pipx && echo "pipx${tab}pipx upgrade-all"
    shimmer_has uv && echo "uv${tab}uv tool upgrade --all"
    shimmer_has flatpak && echo "flatpak${tab}flatpak update --user -y --noninteractive"
    shimmer_has gh && [ -n "$(gh extension list 2>/dev/null)" ] && echo "gh${tab}gh extension upgrade --all"
    shimmer_has ghcup && echo "ghcup${tab}ghcup upgrade"
    shimmer_has tldr && echo "tldr${tab}tldr --update"
    shimmer_has claude && echo "claude${tab}claude update"
    return 0
}

# Why an installed updater can't run without sudo, or nothing. npm's global folder belongs to
# root when Node came from the system's packages.
shimmer_update_needs_sudo() {
    case "$1" in
        npm)
            prefix=$(npm prefix -g 2>/dev/null)
            [ -n "$prefix" ] && [ -d "$prefix/lib" ] && [ ! -w "$prefix/lib" ] && echo "its global folder $prefix belongs to root"
            ;;
    esac
    return 0
}

# The system package manager: an AUR helper first on Arch (it updates the AUR too, and asks
# for sudo itself), then the distribution's own, or macOS's softwareupdate.
shimmer_update_system_tool() {
    if shimmer_macos; then
        echo softwareupdate
        return
    fi
    for tool in paru yay pacman apt-get dnf zypper apk; do
        shimmer_has "$tool" && {
            echo "$tool"
            return
        }
    done
}

# The command that updates the system. `terminal`: you're at the window, so it may ask you
# things. `passwordless`: nobody is, so it must not (sudo -n, and the tool's own "yes").
shimmer_update_system_command() {
    tool=$1
    mode=$2
    if [ "$mode" = "passwordless" ]; then
        case "$tool" in
            paru | yay) cmd="$tool -Syu --noconfirm --sudoflags -n" ;;
            pacman) cmd="sudo -n pacman -Syu --noconfirm" ;;
            apt-get) cmd="sudo -n apt-get update && sudo -n env DEBIAN_FRONTEND=noninteractive apt-get -y upgrade" ;;
            dnf) cmd="sudo -n dnf -y upgrade" ;;
            zypper) cmd="sudo -n zypper --non-interactive update" ;;
            apk) cmd="sudo -n apk upgrade" ;;
            softwareupdate) cmd="sudo -n softwareupdate --install --all" ;;
        esac
        shimmer_has snap && cmd="$cmd && sudo -n snap refresh"
        shimmer_has flatpak && cmd="$cmd && sudo -n flatpak update --system -y --noninteractive"
    else
        case "$tool" in
            paru | yay) cmd="$tool -Syu" ;;
            pacman) cmd="sudo pacman -Syu" ;;
            apt-get) cmd="sudo apt-get update && sudo apt-get upgrade" ;;
            dnf) cmd="sudo dnf upgrade" ;;
            zypper) cmd="sudo zypper update" ;;
            apk) cmd="sudo apk upgrade" ;;
            softwareupdate) cmd="sudo softwareupdate --install --all" ;;
        esac
        shimmer_has snap && cmd="$cmd && sudo snap refresh"
        shimmer_has flatpak && cmd="$cmd && sudo flatpak update --system"
    fi
    echo "$cmd"
}
