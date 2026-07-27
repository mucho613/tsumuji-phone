#!/usr/bin/env bash
set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
    echo "Run this script as root." >&2
    exit 1
fi

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
config_source="${script_dir}/opensips.cfg"
config_target=/etc/opensips/opensips.cfg
timestamp=$(date -u +%Y%m%dT%H%M%SZ)
backup="${config_target}.tsumuji-before-update.${timestamp}"
update_started=no

rollback() {
    exit_code=$?
    trap - ERR
    set +e
    if [[ ${update_started} == yes ]]; then
        cp --archive -- "${backup}" "${config_target}"
        systemctl restart opensips.service
        echo "Update failed; OpenSIPS was restored from ${backup}." >&2
    fi
    exit "${exit_code}"
}

trap rollback ERR

opensips -C -f "${config_source}"
cp --archive -- "${config_target}" "${backup}"
update_started=yes
install -o root -g opensips -m 0640 "${config_source}" "${config_target}"
systemctl restart opensips.service
systemctl is-active --quiet opensips.service

trap - ERR
echo "OpenSIPS configuration updated. Backup: ${backup}"
