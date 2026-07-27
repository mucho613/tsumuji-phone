# OpenSIPS deployment

This configuration places OpenSIPS between tailnet SIP clients and the existing Asterisk
registrar without moving authentication data out of Asterisk.

Network layout:

```text
Tailscale clients -> 100.111.230.4:5060 OpenSIPS
OpenSIPS          -> 100.111.230.4:5061 Asterisk
Asterisk          -> 100.111.230.4:5060 OpenSIPS
OpenSIPS          -> registered Tailscale client contact
```

The Asterisk Tailscale transport must be moved from `100.111.230.4:5060` to
`100.111.230.4:5061` before OpenSIPS starts. Its `local_net` is changed to the
actual LAN (`192.168.1.0/24`), so OpenSIPS is treated as the external signaling
hop and `external_media_address=100.111.230.4` continues to be applied. The LAN
transport on `192.168.1.2:5060` remains unchanged.

Use OpenSIPS 3.6.7 LTS with the SQLite and REST client modules. The Android push
binding is allowed to live for 30 days. One day before it expires, OpenSIPS wakes
the device so it can refresh the binding.

`pn_skip_pn_interval` is intentionally set to `0`. Android tears down its SIP
stack without unregistering after the Push wake window or an incoming call
ends, so a recently refreshed Contact is not proof that the process remains
reachable. Every new incoming request therefore triggers FCM and waits for the
matching re-registration. This makes immediate redial reliable without adding
periodic background traffic.

Run `install-packages.sh` as root first. It installs the pinned packages, initializes
the official `location` SQLite schema, installs `opensips.cfg`, and verifies its
syntax. It deliberately leaves OpenSIPS disabled, so the live SIP port is unchanged.

After reviewing the staged configuration, run `cutover.sh` as root. It:

1. verifies the exact expected Asterisk configuration,
2. saves timestamped copies of every modified system file,
3. moves only the Asterisk tailnet transport to `100.111.230.4:5061`,
4. keeps RTP advertised on the Tailscale address,
5. raises the AOR maximum registration lifetime to 30 days, and
6. starts OpenSIPS on the old client-facing address.

If any cutover check fails, the script stops OpenSIPS, restores Asterisk, and restarts
the previous configuration. The LAN transport on `192.168.1.2:5060` is never changed.

For later `opensips.cfg` changes, run `apply-config.sh` as root. It checks the new
configuration, saves the current file, restarts only OpenSIPS, and restores the
previous file if the restart fails.
