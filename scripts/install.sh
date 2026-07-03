#!/usr/bin/env bash
# Install (or uninstall) fand system-wide. Run from the repo:
#
#   cargo build --release
#   sudo scripts/install.sh          # install/upgrade
#   sudo scripts/install.sh uninstall
#
# Deliberately does NOT enable or start the service — do that yourself once
# the manual hardware test has passed:
#
#   sudo systemctl enable --now fand

set -euo pipefail

BIN_DIR=/usr/local/bin
UNIT_PATH=/etc/systemd/system/fand.service
CONFIG_DIR=/etc/fand
GROUP=fand

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ ${EUID} -ne 0 ]]; then
    echo "error: must run as root (sudo scripts/install.sh)" >&2
    exit 1
fi

uninstall() {
    if systemctl is-active --quiet fand; then
        # Stopping triggers ExecStopPost --restore-auto, handing fans back
        # to firmware before the binary disappears.
        systemctl disable --now fand
    else
        systemctl disable fand 2>/dev/null || true
    fi
    rm -f "${UNIT_PATH}" "${BIN_DIR}/fand" "${BIN_DIR}/fanctl"
    systemctl daemon-reload
    echo "uninstalled. Kept ${CONFIG_DIR}/ and the '${GROUP}' group; remove by hand if wanted."
}

install_all() {
    local release="${repo_dir}/target/release"
    if [[ ! -x "${release}/fand" ]]; then
        # Building under sudo would leave a root-owned target/; build as
        # your own user first.
        echo "error: ${release}/fand not found — run 'cargo build --release' first (not as root)" >&2
        exit 1
    fi

    getent group "${GROUP}" >/dev/null || groupadd --system "${GROUP}"
    # Socket clients (fanctl, GUI) need group membership; takes effect on
    # next login.
    if [[ -n "${SUDO_USER:-}" ]] && ! id -nG "${SUDO_USER}" | tr ' ' '\n' | grep -qx "${GROUP}"; then
        usermod -aG "${GROUP}" "${SUDO_USER}"
        echo "added ${SUDO_USER} to group '${GROUP}' (re-login for it to take effect)"
    fi

    install -Dm755 "${release}/fand" "${BIN_DIR}/fand"
    [[ -x "${release}/fanctl" ]] && install -Dm755 "${release}/fanctl" "${BIN_DIR}/fanctl"

    if [[ ! -e "${CONFIG_DIR}/config.toml" ]]; then
        install -Dm644 "${repo_dir}/config/fand.example.toml" "${CONFIG_DIR}/config.toml"
        echo "installed default config to ${CONFIG_DIR}/config.toml"
    else
        echo "kept existing ${CONFIG_DIR}/config.toml"
    fi

    install -Dm644 "${repo_dir}/systemd/fand.service" "${UNIT_PATH}"
    systemctl daemon-reload

    "${BIN_DIR}/fand" --check --config "${CONFIG_DIR}/config.toml"

    cat <<EOF

installed. Next steps (after the manual hardware test has passed):
  sudo systemctl enable --now fand
  systemctl status fand
EOF
}

case "${1:-install}" in
install) install_all ;;
uninstall) uninstall ;;
*)
    echo "usage: sudo scripts/install.sh [install|uninstall]" >&2
    exit 1
    ;;
esac
