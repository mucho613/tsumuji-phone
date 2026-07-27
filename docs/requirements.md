# 初期リリース要件

## 対象

- Ubuntu サーバー上で常時稼働し、Tailscale から到達できる Asterisk
- Asterisk と同じ Ubuntu サーバー上で OpenSIPS と push gateway を常時稼働
- 内線通話は Asterisk 内で完結し、外線通話は Asterisk がホームゲートウェイへ中継
- 最初の端末は Ubuntu 24.04。次に Windows 11 x64、Android 12/API 31+
- 単一の SIP アカウントを使用し、設定プロファイルの切り替えは行わない
- ユーザ ID は SIP 識別子（AoR）とダイジェスト認証名を兼ねる
- ユーザ ID は半角英数字または `-`、`.`、`_`、`~` の 1～32 文字

## 通話

- 外線／内線の発信と着信、番号表示、応答、拒否、取消、切断
- 発信先番号は半角数字 1～32 桁（先頭の 0 を許可）
- 単一アカウント、単一通話。二つ目の着信には `486 Busy Here`
- クライアントは OpenSIPS の UDP/IPv4 5060 へ接続し、OpenSIPS は localhost の
  Asterisk UDP/IPv4 5061 へ中継
- SIP サーバーには IPv4 アドレスまたは DNS/MagicDNS ホスト名を指定
- デスクトップではローカルアドレスに端末へ割り当てられたプライベート LAN
  または Tailscale の IPv4 アドレスを指定
- Android では Tailscale の IPv4 アドレスを自動取得し、手入力欄や LAN 接続への
  フォールバックは提供しない
- RTP/RTCP は 40000-40009。指定したローカルアドレスへ明示的に bind
- PCMU（G.711 μ-law、8 kHz、20 ms）のみ
- DTMF は RTP telephone-event を優先。未交渉時はインバンドへフォールバック
- ミュート中も無音 RTP を維持
- 110、118、119 は発信禁止

## デスクトップ

- 400x680、最小 360x600 の小型ウィンドウ
- 閉じる操作ではトレイへ格納。明示的な終了メニューを提供
- マイク、通話出力、着信音出力を独立選択。初期値は全て OS 既定
- 通話中のデバイス消失時は通話を終了し、途中切替は行わない
- SIP パスワードは空白を除く印字可能な ASCII 文字 1～128 文字
- OS キーチェーンへ SIP パスワードを一件保存。フロントエンドへ保存値を返さない
- テレメトリー、録音、通話履歴、連絡先は持たない

## Android

- Android 12/API 31 以上を対象とし、実機は `arm64-v8a`、エミュレーターは
  `x86_64` のみをビルド
- 初期配布は署名済み APK の直接配布とし、Google Play での配布は対象外
- Google Play Services と Firebase Cloud Messaging（FCM）を使用
- Tailscale は端末側で常時接続し、SIP、pairing、push gateway の通信は tailnet
  内に限定
- 待受中も SIP 登録を常駐させない。アプリが閉じている場合は FCM の起動信号を
  受信してから一時的に OpenSIPS へ登録
- FCM payload は wake/cancel の種別と推測困難な一時 call token のみに限定し、
  発信者番号、SIP ユーザ ID、SIP 資格情報を含めない
- OpenSIPS は着信時に最大 30 秒間 Android の再登録を待ち、間に合わなければ
  通常の不応答として終了
- 呼が取り消された場合は cancel push により着信 UI と登録処理を終了
- 「待受を開始」は端末を push 対応状態にし、「待受解除」は gateway 上の端末を
  無効化する。保存済み SIP 資格情報は削除しない
- Android Keystore に SIP パスワードと端末資格情報を一件だけ保存し、初回
  pairing は一度限りの pairing code で行う
- 一つの SIP ユーザ ID に対して有効な Android 端末は一台
- 端末再起動後は最初のロック解除後から待受を再開
- Android Service が同一プロセス内の Rust/PJSIP を所有し、UI は状態を監視
- 通話 UI は Core-Telecom の CallStyle 通知を使用し、応答、拒否、切断、
  ミュート、音声経路切り替えを提供
- PJSIP の音声 backend は Oboe とし、端末側は 48 kHz、SIP/RTP は PCMU 8 kHz
- バッテリー最適化の除外を必須にせず、待受中の常駐 foreground service は使用しない

## 初期対象外

保留、転送、通話待ち、複数通話、複数アカウント、設定プロファイル切り替え、
IPv6、映像、FAX、高音質コーデック、自動更新、録音、Google Play 配布、
Google Play Services を持たない Android 端末。
