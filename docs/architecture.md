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
- アカウント設定は RT-500KI、内線 3、固定 SIP/RTP ポート範囲として検証

## 未完のプラットフォーム層

Android では Tauri Kotlin プラグインとして foreground service、
`ConnectionService`、audio focus、通知を実装する予定です。Android の音声経路は
Telecom の動的ルーティングに従い、デスクトップのデバイス ID を共有しません。
