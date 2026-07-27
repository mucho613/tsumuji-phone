#!/usr/bin/env bash
set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
    echo "Run this script as root." >&2
    exit 1
fi

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
config_source="${script_dir}/opensips.cfg"

if [[ ! -f "${config_source}" ]]; then
    echo "opensips.cfg was not found next to this script." >&2
    exit 1
fi

# shellcheck disable=SC1091
source /etc/os-release
if [[ ${VERSION_CODENAME:-} != "noble" ]] ||
   [[ $(dpkg --print-architecture) != "amd64" ]]; then
    echo "This deployment is pinned to Ubuntu 24.04 amd64." >&2
    exit 1
fi

apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y ca-certificates curl sqlite3

install -d -m 0755 /usr/share/keyrings
curl -fsSL https://apt.opensips.org/opensips-org.gpg \
    -o /usr/share/keyrings/opensips-org.gpg
chmod 0644 /usr/share/keyrings/opensips-org.gpg

printf '%s\n' \
    'deb [signed-by=/usr/share/keyrings/opensips-org.gpg] https://apt.opensips.org noble 3.6-releases' \
    > /etc/apt/sources.list.d/opensips.list

apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y \
    opensips=3.6.7-1 \
    opensips-restclient-module=3.6.7-1 \
    opensips-sqlite-module=3.6.7-1 \
    opensips-sqlite-dbschema=3.6.7-1

systemctl disable --now opensips.service
sed -i 's/^RUN_OPENSIPS=.*/RUN_OPENSIPS=no/' /etc/default/opensips

install -o root -g opensips -m 0640 \
    "${config_source}" /etc/opensips/opensips.cfg

install -d -o opensips -g opensips -m 0750 /var/lib/opensips
database=/var/lib/opensips/tsumuji-phone.sqlite
if [[ ! -e "${database}" ]]; then
    runuser -u opensips -- sqlite3 "${database}" <<SQL
.read /usr/share/opensips/sqlite/standard-create.sql
.read /usr/share/opensips/sqlite/usrloc-create.sql
SQL
    chmod 0640 "${database}"
fi

if ! runuser -u opensips -- sqlite3 "${database}" \
    "SELECT 1 FROM sqlite_master WHERE type='table' AND name='location';" |
    grep -qx 1; then
    echo "The OpenSIPS location table is missing." >&2
    exit 1
fi

opensips -C -f /etc/opensips/opensips.cfg
systemctl is-active --quiet asterisk.service

echo "OpenSIPS 3.6.7 is staged but remains disabled."
