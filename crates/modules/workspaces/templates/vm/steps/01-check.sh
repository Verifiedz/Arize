# Supervised: stop with one clear sentence before anything starts.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-vm.sh"

if [ "$VM_SOFTWARE" = "custom" ]; then
    [ -n "$START_COMMAND" ] || shimmer_fail "VM_SOFTWARE is custom: set START_COMMAND in workspace.toml's [env]"
else
    [ -n "$(shimmer_vm_cli "$VM_SOFTWARE")" ] || shimmer_fail "$VM_SOFTWARE isn't installed (or its command-line tool isn't on PATH)"
    shimmer_vm_exists || shimmer_fail "$VM_SOFTWARE has no VM called '$VM_NAME'"
fi

if [ "$REMOTE_EDITOR" != "none" ] || [ "${TERMINAL_APP:-none}" != "none" ]; then
    [ -n "$VM_HOST" ] || shimmer_fail "set VM_HOST (the VM's address) to open an editor or terminal inside it"
    shimmer_has ssh || shimmer_fail "ssh isn't installed"
fi
if [ "$VM_HOST" = "auto" ] && [ "$VM_SOFTWARE" != "utm" ] && [ "$VM_SOFTWARE" != "multipass" ]; then
    shimmer_fail "VM_HOST = auto only works with UTM and Multipass: give the VM's address"
fi
if [ "$REMOTE_EDITOR" != "none" ]; then
    [ -n "$(shimmer_remote_editor_cli "$REMOTE_EDITOR")" ] || shimmer_fail "$REMOTE_EDITOR isn't installed"
fi
shimmer_has_terminal "${TERMINAL_APP:-none}" || shimmer_fail "the terminal '$TERMINAL_APP' isn't installed here"

if [ -n "$SSH_PASSWORD_ITEM" ]; then
    if shimmer_macos; then
        shimmer_has security || shimmer_fail "macOS's security tool is missing"
    else
        shimmer_has secret-tool || shimmer_fail "secret-tool isn't installed (it's in libsecret-tools)"
    fi
    shimmer_secret "$SSH_PASSWORD_ITEM" >/dev/null 2>&1 ||
        shimmer_fail "no keychain entry '$SSH_PASSWORD_ITEM'; save it once with: $(shimmer_secret_how_to_save "$SSH_PASSWORD_ITEM")"
fi
echo "ok: $VM_SOFTWARE VM '$VM_NAME'${VM_HOST:+, ssh to $VM_HOST}"
