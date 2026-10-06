#!/usr/bin/env bash
# Install a `shimmer` launcher into ~/.cargo/bin that runs this checkout's code.
#
# The launcher is a one-command script: `cargo run -q --manifest-path <repo>/Cargo.toml -p shimmer`.
# Every call rebuilds if needed and runs your latest code, from any folder. <repo> is where this
# script lives, not a hard-coded path, so each clone installs a launcher for itself.
#
#   .dev/install-dev-launcher.sh           install; refuses to replace an existing `shimmer`
#   .dev/install-dev-launcher.sh --force   install, replacing whatever `shimmer` is there
#
# Linux and macOS only. See CONTRIBUTING.md.

set -euo pipefail

usage() {
    sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
}

force=0
for arg in "$@"; do
    case "$arg" in
        --force) force=1 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "install-dev-launcher: unknown argument '$arg'" >&2
            usage >&2
            exit 2
            ;;
    esac
done

repo="$(cd "$(dirname "$0")/.." && pwd)"
if [[ ! -f "$repo/Cargo.toml" ]]; then
    echo "install-dev-launcher: no Cargo.toml in $repo; run this script from inside the repo" >&2
    exit 1
fi

bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
target="$bin_dir/shimmer"

if [[ -e "$target" || -L "$target" ]] && [[ $force -eq 0 ]]; then
    echo "install-dev-launcher: $target already exists; not replacing it." >&2
    echo "  Re-run with --force to replace it. What is there now:" >&2
    head -n 5 "$target" 2>/dev/null | sed 's/^/    /' >&2 || true
    exit 1
fi

mkdir -p "$bin_dir"

# Write next to the target and rename, so an interrupted install never leaves half a launcher.
tmp="$(mktemp "$bin_dir/.shimmer.XXXXXX")"
trap 'rm -f "$tmp"' EXIT
{
    echo '#!/usr/bin/env bash'
    echo "# Dev launcher for the Shimmer checkout at $repo. Installed by .dev/install-dev-launcher.sh."
    printf 'exec cargo run -q --manifest-path %q -p shimmer -- "$@"\n' "$repo/Cargo.toml"
} >"$tmp"
chmod 755 "$tmp"
mv -f "$tmp" "$target"
trap - EXIT

echo "Installed $target -> $repo"

case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) echo "Note: $bin_dir is not on your PATH. Add it (rustup normally does) to run 'shimmer' anywhere." ;;
esac
