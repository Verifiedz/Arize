# Supervised: download AI_MODEL with ollama, for a coding assistant with no internet. Starts
# ollama's server for the download if it isn't running, and stops it after.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

shimmer_offline_skipped ai && exit 0
[ -n "$AI_MODEL" ] || exit 0
if ! shimmer_has ollama; then
    shimmer_offline_result warn ai "AI_MODEL is set, but ollama isn't installed: https://ollama.com"
    exit 0
fi
started=""
if ! ollama list </dev/null >/dev/null 2>&1; then
    ollama serve </dev/null >"$SHIMMER_STATE_DIR/ollama.log" 2>&1 &
    started=$!
    i=0
    while ! ollama list </dev/null >/dev/null 2>&1 && [ $i -lt 30 ]; do
        sleep 1
        i=$((i + 1))
    done
fi
echo "== ollama pull $AI_MODEL"
if ollama pull "$AI_MODEL" </dev/null; then
    shimmer_offline_result ok ai "$AI_MODEL downloaded: ollama run $AI_MODEL"
else
    shimmer_offline_result warn ai "couldn't download $AI_MODEL (see the ai step's log)"
fi
[ -n "$started" ] && kill "$started" 2>/dev/null
exit 0
