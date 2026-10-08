# Detached: open a folder inside the VM in Cursor or VS Code, over SSH (Remote-SSH). The editor
# asks for a password itself if there's no SSH key.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-vm.sh"

[ "${REMOTE_EDITOR:-none}" = "none" ] && exit 0
host=$(shimmer_vm_host)
[ -n "$host" ] || exit 0
target=$(shimmer_ssh_target "$host")
# Remote-SSH takes the port from ~/.ssh/config, not from the address.
[ "${SSH_PORT:-22}" = "22" ] || echo "port $SSH_PORT: add it to a Host entry in ~/.ssh/config for the editor"
cli=$(shimmer_remote_editor_cli "$REMOTE_EDITOR")
shimmer_record_left_open "the $REMOTE_EDITOR window connected to the VM (the editor keeps all its windows in one program)"
if [ -n "$REMOTE_FOLDER" ]; then
    "$cli" --remote "ssh-remote+$target" "$REMOTE_FOLDER" >/dev/null 2>&1 &
else
    "$cli" --remote "ssh-remote+$target" >/dev/null 2>&1 &
fi
