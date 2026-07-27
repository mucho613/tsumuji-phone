#!/usr/bin/env bash
set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
    echo "Run this script as root." >&2
    exit 1
fi

asterisk_config=/etc/asterisk/pjsip.conf
opensips_defaults=/etc/default/opensips
timestamp=$(date -u +%Y%m%dT%H%M%SZ)
asterisk_backup="${asterisk_config}.tsumuji-before-opensips.${timestamp}"
opensips_defaults_backup="${opensips_defaults}.tsumuji-before-cutover.${timestamp}"
generated_config=$(mktemp)
cutover_started=no

cleanup() {
    rm -f -- "${generated_config}"
}

rollback() {
    exit_code=$?
    trap - ERR
    set +e
    if [[ ${cutover_started} == yes ]]; then
        systemctl disable --now opensips.service
        cp --archive -- "${asterisk_backup}" "${asterisk_config}"
        cp --archive -- "${opensips_defaults_backup}" "${opensips_defaults}"
        systemctl restart asterisk.service
        echo "Cutover failed; Asterisk was restored from ${asterisk_backup}." >&2
    fi
    cleanup
    exit "${exit_code}"
}

trap cleanup EXIT
trap rollback ERR

opensips -C -f /etc/opensips/opensips.cfg
systemctl is-active --quiet asterisk.service

[[ $(grep -c '^bind=100\.111\.230\.4:5060$' "${asterisk_config}") -eq 1 ]]
[[ $(grep -c '^local_net=100\.111\.230\.4/32$' "${asterisk_config}") -eq 1 ]]
[[ $(grep -c '^remove_existing=yes$' "${asterisk_config}") -eq 4 ]]
[[ $(grep -c '^maximum_expiration=' "${asterisk_config}" || true) -eq 0 ]]

awk '
    $0 == "bind=100.111.230.4:5060" {
        print "bind=100.111.230.4:5061"
        next
    }
    $0 == "local_net=100.111.230.4/32" {
        print "local_net=192.168.1.0/24"
        next
    }
    $0 == "remove_existing=yes" {
        print
        print "maximum_expiration=2592000"
        next
    }
    { print }
' "${asterisk_config}" > "${generated_config}"

cp --archive -- "${asterisk_config}" "${asterisk_backup}"
cp --archive -- "${opensips_defaults}" "${opensips_defaults_backup}"
cutover_started=yes

install -o root -g asterisk -m 0640 \
    "${generated_config}" "${asterisk_config}"
sed -i 's/^RUN_OPENSIPS=.*/RUN_OPENSIPS=yes/' "${opensips_defaults}"

# A reload validates the endpoint and AOR options. The transport itself is
# intentionally applied by the following full restart.
asterisk -rx "core reload" >/dev/null
systemctl restart asterisk.service
systemctl is-active --quiet asterisk.service

systemctl enable --now opensips.service
systemctl is-active --quiet opensips.service

ss -lun | grep -q '100\.111\.230\.4:5060'
ss -lun | grep -q '100\.111\.230\.4:5061'
ss -lun | grep -q '192\.168\.1\.2:5060'

trap - ERR
echo "Cutover completed. Asterisk backup: ${asterisk_backup}"
