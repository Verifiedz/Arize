# Supervised: what to fix while you still can: battery, disk space, and logins that expire.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

shimmer_offline_skipped machine && exit 0

# Battery: plug in before the downloads.
charge=""
if shimmer_macos; then
    batt=$(pmset -g batt 2>/dev/null)
    charge=$(printf '%s' "$batt" | grep -o '[0-9]*%' | head -n 1 | tr -d %)
    printf '%s' "$batt" | grep -q 'AC Power' && plugged=yes || plugged=no
else
    for bat in /sys/class/power_supply/BAT*; do
        [ -r "$bat/capacity" ] || continue
        charge=$(cat "$bat/capacity")
        [ "$(cat "$bat/status" 2>/dev/null)" = "Discharging" ] && plugged=no || plugged=yes
        break
    done
fi
if [ -n "$charge" ]; then
    if [ "$charge" -lt 80 ] && [ "$plugged" = no ]; then
        shimmer_offline_result warn machine "battery at $charge% and not charging: plug in"
    else
        shimmer_offline_result ok machine "battery at $charge%$([ "$plugged" = yes ] && echo ', charging')"
    fi
fi

# Disk space: the downloads need room.
avail=$(df -Pk "$HOME" 2>/dev/null | awk 'NR == 2 { print $4 }')
if [ -n "$avail" ]; then
    gb=$((avail / 1048576))
    if [ "$gb" -lt 5 ]; then
        shimmer_offline_result warn machine "only $gb GB free: downloads may run out of room (a free-disk workspace helps)"
    else
        shimmer_offline_result ok machine "$gb GB free"
    fi
fi

# Logins: refresh any that has expired now, while there's network to do it.
login() {
    if shimmer_offline_limit 20 sh -c "$2" >/dev/null 2>&1; then
        shimmer_offline_result ok machine "$1 login works"
    else
        shimmer_offline_result warn machine "$1 login has expired or isn't set up: $3"
    fi
}
shimmer_has gh && login gh "gh auth status" "gh auth login"
shimmer_has aws && [ -f "$HOME/.aws/config" ] && login aws "aws sts get-caller-identity" "aws sso login (or aws configure)"
shimmer_has gcloud && login gcloud "gcloud auth print-access-token" "gcloud auth login"
shimmer_has az && login az "az account get-access-token" "az login"
exit 0
