# Helpers for the free-disk template (ADR 0025): what can be cleaned, how big it is, and the
# record of what was freed. Steps load it after shimmer-open.sh:
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-free.sh"
#
# Only things that come back by themselves are removed: build folders (a rebuild or reinstall
# makes them again) and download caches (a tool downloads again what it needs). Never your
# files, git repos, virtualenvs, the trash, or docker volumes.
#
# POSIX sh only. This is your copy: Shimmer never changes it after the workspace is created.

SHIMMER_RESULTS="$SHIMMER_STATE_DIR/results"

shimmer_free_result() {
    mkdir -p "$SHIMMER_STATE_DIR"
    echo "$*" >>"$SHIMMER_RESULTS"
}

shimmer_free_skipped() {
    case " $SKIP " in *" $1 "*) return 0 ;; esac
    return 1
}

shimmer_free_preview() {
    [ "${MODE:-clean}" = "preview" ]
}

# Kilobytes in a folder (0 when it's missing).
shimmer_free_size() {
    [ -e "$1" ] || {
        echo 0
        return
    }
    du -sk "$1" 2>/dev/null | awk '{ print $1 }'
}

# Kilobytes free on the disk holding a path.
shimmer_free_avail() {
    df -Pk "${1:-$HOME}" 2>/dev/null | awk 'NR == 2 { print $4 }'
}

# 1536 → "1.5 MB", for kilobytes.
shimmer_free_human() {
    awk -v kb="${1:-0}" 'BEGIN {
        if (kb >= 1048576) printf "%.1f GB", kb / 1048576
        else if (kb >= 1024) printf "%.0f MB", kb / 1024
        else printf "%d KB", kb
    }'
}

# CODE_DIRS with each ~ expanded, the ones that exist, one per line.
shimmer_free_code_dirs() {
    for d in ${CODE_DIRS:-~/code}; do
        case "$d" in
            "~") d=$HOME ;;
            "~/"*) d="$HOME/${d#"~/"}" ;;
        esac
        [ -d "$d" ] && echo "$d"
    done
    return 0
}

# Has nothing in project folder $1 changed for UNTOUCHED_DAYS days? Build folders, dependency
# folders and .git don't count: they change on their own.
shimmer_free_untouched() {
    recent=$(find "$1" \( -name target -o -name node_modules -o -name .git \) -prune -o \
        -type f -mtime "-${UNTOUCHED_DAYS:-30}" -print 2>/dev/null | head -n 1)
    [ -z "$recent" ]
}

# Every build folder under CODE_DIRS that may go, as "kind<TAB>folder" lines: a Rust target/
# Cargo itself marked (CACHEDIR.TAG), and a node_modules/ next to a package.json. Hidden folders
# and the insides of these folders are not searched.
shimmer_free_build_folders() {
    tab=$(printf '\t')
    # Find the manifests (find never goes inside the folders themselves), then look next to each.
    shimmer_free_code_dirs | while IFS= read -r root; do
        find "$root" -mindepth 1 -maxdepth 6 \( -name '.*' -o -name node_modules -o -name target \) -prune -o \
            \( -name Cargo.toml -o -name package.json \) -type f -print 2>/dev/null
    done | while IFS= read -r manifest; do
        dir=$(dirname "$manifest")
        case "$manifest" in
            */Cargo.toml) [ -f "$dir/target/CACHEDIR.TAG" ] && echo "rust-targets${tab}$dir/target" ;;
            */package.json) [ -d "$dir/node_modules" ] && [ ! -L "$dir/node_modules" ] && echo "node-modules${tab}$dir/node_modules" ;;
        esac
    done | sort -u
}

# The folder a tool says its cache is in, or "-" when it doesn't say.
shimmer_free_dir() {
    d=$("$@" 2>/dev/null | head -n 1)
    echo "${d:--}"
}

# Every installed tool cache, as "name<TAB>folder<TAB>clean command" lines. The folder is what
# gets measured; "-" when there's none, measured by the disk's free space instead (never empty:
# `read` would merge two tabs in a row and shift the command into the folder).
shimmer_free_caches() {
    tab=$(printf '\t')
    shimmer_has npm && echo "npm${tab}$HOME/.npm/_cacache${tab}npm cache clean --force"
    shimmer_has pnpm && echo "pnpm${tab}$(shimmer_free_dir pnpm store path)${tab}pnpm store prune"
    shimmer_has yarn && echo "yarn${tab}$(cd "$HOME" && shimmer_free_dir yarn cache dir)${tab}yarn cache clean"
    shimmer_has bun && echo "bun${tab}${BUN_INSTALL_CACHE_DIR:-$HOME/.bun/install/cache}${tab}bun pm cache rm"
    python3 -m pip --version >/dev/null 2>&1 && echo "pip${tab}$(shimmer_free_dir python3 -m pip cache dir)${tab}python3 -m pip cache purge"
    shimmer_has uv && echo "uv${tab}$(shimmer_free_dir uv cache dir)${tab}uv cache prune"
    shimmer_has go && echo "go${tab}$(shimmer_free_dir go env GOCACHE)${tab}go clean -cache"
    shimmer_has cargo-cache && echo "cargo${tab}${CARGO_HOME:-$HOME/.cargo}/registry${tab}cargo-cache --autoclean"
    shimmer_has brew && echo "brew${tab}$(shimmer_free_dir brew --cache)${tab}brew cleanup --prune=all"
    shimmer_has docker && docker info >/dev/null 2>&1 && echo "docker${tab}-${tab}docker system prune -f"
    if ! shimmer_macos && [ -d "$HOME/.cache/thumbnails" ]; then
        echo "thumbnails${tab}$HOME/.cache/thumbnails${tab}rm -rf $(shimmer_quote "$HOME/.cache/thumbnails")"
    fi
    return 0
}

# The system's package cache and old logs, which need sudo. `terminal` may ask you things;
# `passwordless` must not.
shimmer_free_system_command() {
    mode=$1
    if [ "$mode" = "passwordless" ]; then sudo="sudo -n"; else sudo="sudo"; fi
    cmd=""
    if shimmer_has paccache; then
        cmd="$sudo paccache -rk1 && $sudo paccache -ruk0"
    elif shimmer_has pacman; then
        cmd="$sudo pacman -Sc --noconfirm"
    elif shimmer_has apt-get; then
        cmd="$sudo apt-get clean"
    elif shimmer_has dnf; then
        cmd="$sudo dnf clean all"
    elif shimmer_has zypper; then
        cmd="$sudo zypper clean --all"
    fi
    if shimmer_has journalctl; then
        cmd="${cmd:+$cmd && }$sudo journalctl --vacuum-time=2weeks"
    fi
    echo "$cmd"
}
