# Supervised: save SAVE_PAGES for reading offline, one HTML file each in OFFLINE_DIR/pages: with
# monolith (the page and its images in one file) when it's installed, else wget (the page and
# what it needs, in a folder), else curl (the page alone).
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

shimmer_offline_skipped pages && exit 0
[ -n "$SAVE_PAGES" ] || exit 0
dir="$(shimmer_offline_dir)/pages"
mkdir -p "$dir"
for url in $SAVE_PAGES; do
    file="$dir/$(shimmer_offline_page_name "$url")"
    echo "== $url"
    if shimmer_has monolith; then
        monolith "$url" -o "$file" </dev/null
    elif shimmer_has wget; then
        sub="$dir/$(shimmer_offline_page_name "$url" | sed 's/\.html$//')"
        wget -q --page-requisites --convert-links --adjust-extension --span-hosts -e robots=off -P "$sub" "$url" </dev/null
        page=$(find "$sub" -name '*.html' 2>/dev/null | head -n 1)
        # A small page that opens the saved copy, so every page is one file in pages/.
        [ -n "$page" ] && printf '<!doctype html><meta http-equiv="refresh" content="0; url=%s">\n' "${page#"$dir/"}" >"$file"
    else
        curl -fsSL "$url" -o "$file" </dev/null
    fi
    if [ -s "$file" ]; then
        shimmer_offline_result ok pages "saved $url"
    else
        rm -f "$file"
        shimmer_offline_result warn pages "couldn't save $url"
    fi
done
shimmer_has monolith || shimmer_offline_result info pages "install monolith for pages saved as one file with their images"
exit 0
