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
- SIP サーバーは IPv4 または DNS/MagicDNS ホスト名、ローカルアドレスは
  プライベート LAN または Tailscale の IPv4 として検証
- ユーザ ID は SIP 識別子（AoR）とダイジェスト認証名を兼ね、半角英数字または
  `-`、`.`、`_`、`~` の 1～32 文字として検証
- パスワードは空白を除く印字可能な ASCII 1～128 文字として検証
- SIP/RTP ポート範囲は固定し、音声コーデックは PCMU のみに限定
- SIP パスワードの資格情報ストアは一件のみとし、複数アカウントの同時登録や
  プロファイル切り替えは扱わない

## 未完のプラットフォーム層

Android では Tauri Kotlin プラグインとして foreground service、
`ConnectionService`、audio focus、通知を実装する予定です。Android の音声経路は
Telecom の動的ルーティングに従い、デスクトップのデバイス ID を共有しません。
