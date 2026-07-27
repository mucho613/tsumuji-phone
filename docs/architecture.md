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

デスクトップと Android は、どちらも tailnet 内の Asterisk UDP 5060 へ直接
接続します。RTP もクライアントと Asterisk の間で直接送受信します。

```text
Desktop / Android
    | SIP: Tailscale UDP 5060
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

画面を開いている間は、Android Service が SIP 登録を維持します。「待受を開始」した
意図、SIP サーバー、ユーザ ID を端末内へ保存し、明示的に「待受を停止」するまで
維持します。プロセスの次回通常起動時は保存済みパスワードを Android Keystore
から読み、一度だけ自動登録を開始します。画面を閉じて Service が終了した後は
SIP登録を常駐させません。

初回登録は最大 10 秒待ちます。成功または明示的な失敗は直ちに確定し、失敗後は
自動再試行せず「再接続」を待ちます。登録後に接続を失った場合だけ 5 秒間隔、
最大 60 秒で再登録を試し、復旧できなければ手動再接続へ移行します。進行中の通話は
この復旧処理から独立して維持します。

設定差分がある場合は「変更を適用」で現在の登録を停止して新しい設定で開始します。
通話中は変更適用、待受停止、再接続を禁止します。登録中は番号入力を許可しますが、
発信を予約せず、登録完了まで発信操作を無効にします。

### FCM 起動試験

現フェーズは実着信を扱わず、アプリ終了中に FCM がプロセスを起動し、画面を開かず
PJSIP 登録を完了できるかを縦切りで検証します。

```text
Windows test sender
    | HTTPS: FCM HTTP v1 (FID, high priority, TTL 30 s)
    v
Firebase Cloud Messaging
    | data-only: type=wake, callToken
    v
FirebaseMessagingService
    | startForegroundService
    v
TelephonyService -- Tailscale/SIP --> Asterisk
    |
    +-- success or failure, then unconditional stop by 30 s
```

FCM SDK は registration token ではなく FID の直接送信 API を使用します。
`onRegistered(installationId)` で受け取った FID は専用の Android Keystore 鍵で
暗号化し、Android native 内だけで扱います。ローカル送信時も FID はコマンドライン
引数にせず、実行後の入力欄へ貼り付けます。

FCM callback は高優先度、data key の完全一致、通知権限、待受意図、call token の
形式と重複を検証します。call token の生値は Service へ渡さず、SHA-256 hash だけを
2分保持します。登録済み／登録中なら foreground service を開始しません。

有効な wake は `shortService` 型 foreground service を起動し、即時に
「着信接続中」を表示します。Tauri Activity を作らないプロセス起動にも対応するため、
Android Service と Tauri UI は遅延初期化した同じ Rust/PJSIP engine を共有します。
登録応答は10秒待ち、起動信号の受信から30秒で成功・失敗・通話状態にかかわらず
PJSIP と foreground service を停止します。

Push gateway、pairing、OpenSIPS、INVITE 保留、cancel、実着信は後続設計まで
実装しません。

## Android プラットフォーム層

Android Service が同一プロセス内の Rust/PJSIP engine を所有し、Tauri UI は
service の状態を監視します。ユーザー向けの待受状態は「待受停止」「接続中」
「待受中」「接続失敗」の四つに統一します。通話の有無と段階は待受状態とは別に
`PhoneSnapshot` で表します。

SIP パスワードは Android Keystore で保護し、保存先は一件だけとします。FID は
別の Android Keystore 鍵で保護します。ローカル SIP アドレスは
Tailscale IPv4 から自動決定し、設定画面へ入力欄を出しません。明示的な待受停止では
自動開始の意図を解除しますが、保存済み SIP サーバー、ユーザ ID、パスワードは維持
します。FID は待受停止時も維持します。

PJSIP は Oboe を使用し、端末音声48 kHz と PCMU/RTP 8 kHz の変換を担当します。
バッテリー最適化の除外は要求しません。Core-Telecom と CallStyle 通知は実着信
フェーズまで延期します。

## 配布対象

- 最小 API: 31（Android 12）
- 実機 ABI: `arm64-v8a`
- エミュレーター ABI: `x86_64`
- 初期配布: 直接配布する署名済み APK
- push: Google Play Services の FCM のみ
