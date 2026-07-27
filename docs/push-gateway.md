# Push gateway 開発・配置手順

## 役割

`tsumuji-push-gateway` は、OpenSIPS が指定したランダム端末 ID を Firebase
Installation ID（FID）へ変換し、FCM の起動信号を送ります。SIP の INVITE や音声は
中継しません。

公開範囲は次の二つに分離します。

- `127.0.0.1:8787`: pairing と FID 更新。Tailscale Serve で HTTPS 公開
- `127.0.0.1:8788`: wake。OpenSIPS と同じ Ubuntu サーバーからのみ使用

gateway は loopback 以外のアドレスを指定すると起動を拒否します。

## Ubuntu 24.04 での build

```bash
cargo build --release -p tsumuji-push-gateway
install -m 0755 target/release/tsumuji-push-gateway \
  ~/.local/bin/tsumuji-push-gateway
install -d -m 0700 ~/.local/share/tsumuji-push-gateway ~/.config/tsumuji-phone
install -m 0600 \
  /path/to/firebase-service-account.json \
  ~/.config/tsumuji-phone/firebase-service-account.json
```

サービスアカウントには FCM HTTP v1 送信に必要な最小権限だけを付与します。

## systemd

repository の `deploy/tsumuji-push-gateway.service` をユーザーサービスとして
配置します。

```bash
install -Dm 0644 deploy/tsumuji-push-gateway.service \
  ~/.config/systemd/user/tsumuji-push-gateway.service
systemctl --user daemon-reload
systemctl --user enable --now tsumuji-push-gateway
sudo loginctl enable-linger "$USER"
curl --fail http://127.0.0.1:8787/healthz
curl --fail http://127.0.0.1:8788/healthz
```

`enable-linger` は、SSH セッション終了後と再起動後もユーザーサービスを常時稼働
させるために一度だけ実行します。

## Tailscale Serve

Ubuntu サーバーの MagicDNS 名へ tailnet 内限定の HTTPS を設定します。

```bash
sudo tailscale set --operator="$USER"
tailscale serve --bg 8787
tailscale serve status
```

operator 設定は初回だけ必要です。gateway 自体は引き続き loopback で待ち受け、
Tailscale Serve だけが tailnet から到達できる HTTPS を終端します。

Android の「Push gateway URL」には、表示された
`https://<server>.<tailnet>.ts.net` を入力します。HTTP は使用できません。

## ペアリング

Android の FCM が「準備完了」になった後、SIP ユーザに対する一度限りのコードを
Ubuntu で発行します。

```bash
~/.local/bin/tsumuji-push-gateway pairing-code \
  --database ~/.local/share/tsumuji-push-gateway/gateway.sqlite3 \
  --sip-user ts-line3
```

表示された `ABCDE-23456` 形式のコードを10分以内に Android へ入力します。成功時は
コードが直ちに無効になり、同じ SIP ユーザの古い端末対応は置き換えられます。

## gateway 単体の wake 確認

設定画面に表示されるランダム端末 ID を使います。FID や資格情報は入力しません。

```bash
curl --fail \
  --request POST \
  --header 'Content-Type: application/json' \
  --data '{"deviceId":"<random-device-id>"}' \
  http://127.0.0.1:8788/internal/v1/wake
```

応答が `{"status":"sent"}` なら FCM への送信を受け付けています。FID、端末資格情報、
call token、サービスアカウント鍵をログやコマンドラインへ出さないでください。
