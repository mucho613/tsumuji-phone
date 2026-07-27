# アーキテクチャ

## 境界

`telephony-core` がアプリから見える安定した契約です。SIP 実装の型やポインターを
この境界より外へ出しません。`telephony-pjsip` だけが `unsafe` を許可し、
`native/pjsip-adapter` の小さな C ABI を呼び出します。将来 rvoip へ移行する場合は
`TelephonyEngine` の別実装として追加します。

PJSUA-LIB のコールバックは C アダプターから Rust FFI の同期済み状態機械へ通知
されます。アプリ UI は `PhoneSnapshot` のみを受け取り、PJSIP のスレッドや
ライフタイムを扱いません。

## セキュリティ

- Tauri capability はメインウィンドウの `core:default` のみ
- CSP はローカルアセットと Tauri IPC に限定
- 任意のファイル、シェル、外部 URL を開くプラグインは使用しない
- SIP パスワードは Rust 側から OS キーチェーンへ保存し、IPC 応答には含めない
- SIP サーバーは IPv4 または DNS/MagicDNS ホスト名として検証
- 通常アプリのローカルアドレスは列挙した `100.64.0.0/10` の IPv4 から一意に
  自動決定し、0 件または複数件の場合は接続しない
- ユーザ ID は SIP 識別子（AoR）とダイジェスト認証名を兼ね、半角英数字または
  `-`、`.`、`_`、`~` の 1～32 文字として検証
- パスワードは空白を除く印字可能な ASCII 1～128 文字として検証
- SIP/RTP ポート範囲は固定し、音声コーデックは PCMU のみに限定
- SIP パスワードの資格情報ストアは一件のみとし、複数アカウントの同時登録や
  プロファイル切り替えは扱わない

## SIP 経路

デスクトップと Android は、どちらも tailnet 内の OpenSIPS UDP 5060 を SIP の
入口とします。OpenSIPS は同じ Ubuntu サーバーの Asterisk localhost UDP 5061
へ SIP を中継します。RTP は OpenSIPS や push gateway を経由せず、クライアントと
Asterisk の間で直接送受信します。

```text
Desktop / Android
    | SIP: Tailscale UDP 5060
    v
OpenSIPS
    | SIP: localhost UDP 5061
    v
Asterisk -------- Home gateway / external line
    ^
    | RTP: direct, PCMU
    +---------------- Desktop / Android
```

デスクトップは OS のネットワークインターフェースを列挙し、Android は Java の
`NetworkInterface` を列挙します。どちらも `100.64.0.0/10` に属する一意の IPv4
だけを SIP/RTP の bind アドレスとして使用します。設定画面には手入力欄を出しません。
診断用 CLI は任意のインターフェースを試験できるよう明示指定を維持します。

設定画面は最後に適用した SIP サーバーとユーザ ID を基準として保持します。現在値に
差分があるか、新しいパスワードが入力された場合だけ「変更を適用」を表示します。
値を元へ戻した場合は非表示にし、登録開始を受け付けた時点で新しい基準へ更新します。

## Android の待受

現段階では、Android Service が存続している間は SIP 登録を維持します。「待受を開始」
した意図、SIP サーバー、ユーザ ID を端末内へ保存し、明示的に「待受を停止」するまで
維持します。プロセスの次回起動時は保存済みパスワードを Android Keystore から読み、
一度だけ自動登録を開始します。

初回登録は最大 10 秒待ちます。成功または明示的な失敗は直ちに確定し、失敗後は
自動再試行せず「再接続」を待ちます。登録後に接続を失った場合だけ 5 秒間隔、
最大 60 秒で再登録を試し、復旧できなければ手動再接続へ移行します。進行中の通話は
この復旧処理から独立して維持します。

設定差分がある場合は「変更を適用」で現在の登録を停止して新しい設定で開始します。
通話中は変更適用、待受停止、再接続を禁止します。登録中は番号入力を許可しますが、
発信を予約せず、登録完了まで発信操作を無効にします。

### Push 対応後

バッテリー消費を抑えるため、アプリを閉じている間は SIP 登録や foreground service
を常駐させません。OpenSIPS の RFC 8599 push 対応 contact と FCM を使い、着信時
だけアプリを起動します。この段階で、現在のプロセス存続中の登録維持方針を見直します。

1. 「待受を開始」で Android は push token と一時的な SIP 登録を OpenSIPS へ通知し、
   push gateway 上の端末を有効化する
2. 着信時、OpenSIPS は Android の再登録を最大 30 秒待ちながら loopback API で
   push gateway へ wake を要求する
3. push gateway は FCM へ wake と推測困難な一時 call token だけを送る
4. Android Service が起動し、Tailscale IPv4 を取得して OpenSIPS へ SIP 登録する
5. OpenSIPS は保留していた INVITE を新しい contact へ配送する
6. 呼が先に終了した場合、gateway は cancel push を送り、Android は着信表示と
   一時登録を終了する

FCM は SIP の signaling 経路ではなく、起動信号の配送だけに使用します。発信者番号、
SIP ユーザ ID、SIP 資格情報は payload に含めません。Android が INVITE を受信した
後に、SIP signaling から着信情報を表示します。OpenSIPS、Asterisk、push gateway
および Android API は Tailscale からだけ到達可能とします。

## Push gateway

push gateway はこのリポジトリ内の Rust workspace member として実装し、Ubuntu の
systemd service として OpenSIPS と同じサーバーで常時稼働させます。

- OpenSIPS からの wake/cancel API は loopback のみに bind
- Android の pairing/device API は Tailscale Serve の HTTPS 経由で公開
- device、FCM token、push 有効状態、一時 call token を SQLite に保存
- 初回登録は短時間だけ有効な一度限りの pairing code と端末鍵で認証
- 一つの SIP ユーザ ID に対して有効な Android device は一台
- ログへ SIP パスワード、端末秘密鍵、完全な電話番号を出力しない

## Android プラットフォーム層

Android Service が同一プロセス内の Rust/PJSIP engine を所有し、Tauri UI は
service の状態を監視します。ユーザー向けの待受状態は「待受停止」「接続中」
「待受中」「接続失敗」の四つに統一します。通話の有無と段階は待受状態とは別に
`PhoneSnapshot` で表します。

SIP パスワードは Android Keystore で保護し、保存先は一件だけとします。Push 対応後
に追加する device credential も同様に保護します。ローカル SIP アドレスは
Tailscale IPv4 から自動決定し、設定画面へ入力欄を出しません。明示的な待受停止では
自動開始の意図を解除しますが、保存済み SIP サーバー、ユーザ ID、パスワードは維持
します。Push 対応後は端末再起動後の最初のロック解除を経て push 待受を復元し、
待受停止を gateway にも反映します。

着信と通話操作は Core-Telecom と CallStyle 通知へ統合します。音声 focus、
Bluetooth、スピーカー、受話口、ミュートを Android の動的 routing に従って扱い、
デスクトップの音声 device ID は共有しません。PJSIP は Oboe を使用し、端末音声
48 kHz と PCMU/RTP 8 kHz の変換を担当します。バッテリー最適化の除外は要求しません。

## 配布対象

- 最小 API: 31（Android 12）
- 実機 ABI: `arm64-v8a`
- エミュレーター ABI: `x86_64`
- 初期配布: 直接配布する署名済み APK
- push: Google Play Services の FCM のみ
