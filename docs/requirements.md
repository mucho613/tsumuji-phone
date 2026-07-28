# 現行要件

## 対象

- Ubuntu サーバー上で常時稼働し、Tailscale から到達できる Asterisk
- 内線通話は Asterisk 内で完結し、外線通話は Asterisk がホームゲートウェイへ中継
- 最初の端末は Ubuntu 24.04。次に Windows 11 x64、Android 12/API 31+
- 単一の SIP アカウントを使用し、設定プロファイルの切り替えは行わない
- ユーザ ID は SIP 識別子（AoR）とダイジェスト認証名を兼ねる
- ユーザ ID は半角英数字または `-`、`.`、`_`、`~` の 1～32 文字

## 通話

- 外線／内線の発信と着信、番号表示、応答、拒否、取消、切断
- 発信先番号は半角数字 1～32 桁（先頭の 0 を許可）
- 単一アカウント、単一通話。二つ目の着信には `486 Busy Here`
- クライアントは Asterisk の UDP/IPv4 5060 へ直接接続
- SIP サーバーには IPv4 アドレスまたは DNS/MagicDNS ホスト名を指定
- デスクトップと Android は `100.64.0.0/10` の Tailscale IPv4 アドレスを自動取得し、
  手入力欄や LAN 接続へのフォールバックを提供しない
- Tailscale IPv4 が 0 件または複数件の場合は接続を開始せず、理由を表示する
- RTP/RTCP は 40000-40009。自動取得した Tailscale IPv4 へ明示的に bind
- PCMU（G.711 μ-law、8 kHz、20 ms）のみ
- DTMF は RTP telephone-event（RFC 4733）のみ
- ミュート中も無音 RTP を維持
- 発信・着信とも通話成立時から単調時計で経過時間を計り、電話画面の相手番号直下へ
  1秒単位で表示する。1時間未満は `MM:SS`、1時間以上は `H:MM:SS` とし、
  Android の CallStyle 通知には表示しない
- 110、118、119 は発信禁止

## デスクトップ

- 400x680、最小 360x600 の小型ウィンドウ
- 閉じる操作ではトレイへ格納。明示的な終了メニューを提供
- マイク、通話出力、着信音出力を独立選択。初期値は全て OS 既定
- 通話中のデバイス消失時は通話を終了し、途中切替は行わない
- SIP パスワードは空白を除く印字可能な ASCII 文字 1～128 文字
- OS キーチェーンへ SIP パスワードを一件保存。フロントエンドへ保存値を返さない
- 待受開始後の「変更を適用」は、SIP サーバー、ユーザ ID、または新しいパスワード
  に適用済み設定との差分がある場合だけ表示
- テレメトリー、録音、通話履歴、連絡先は持たない

## Android

- Android 12/API 31 以上を対象とし、実機は `arm64-v8a`、エミュレーターは
  `x86_64` のみをビルド
- 初期配布は署名済み APK の直接配布とし、Google Play での配布は対象外
- Tailscale は Android の「常時接続 VPN」として端末側で自動起動し、SIP 通信は
  tailnet 内に限定する。「VPN なしの接続をブロック」は必須としない
- SIP ユーザ ID は端末ごとに専用とし、別の SIP クライアントと資格情報を共有しない
- Android Keystore に SIP パスワードを一件だけ保存し、SIP サーバーとユーザ ID
  も端末内へ保存する。UI や IPC へ保存済みパスワードを返さない
- 「待受を開始」後は明示的に「待受を停止」するまで待受設定を維持し、アプリの
  次回起動時に保存済み設定で SIP 登録を一度自動開始
- 初回登録は明示的な成功または失敗を直ちに反映し、応答がない場合は 10 秒で
  「接続失敗」とする。同一起動中の初回失敗後は自動再試行しない
- 「接続失敗」では理由と「再接続」を表示し、手動再接続は保存済み設定を直ちに使う
- 登録完了後に接続を失った場合だけ、5 秒間隔、最大 60 秒で自動復旧を試す。
  復旧できなければ手動再接続へ移行し、進行中の通話は切断しない
- ユーザー向けの待受状態は「待受停止」「接続中」「待受中」「接続失敗」の四つ
- 接続中も番号入力は許可するが、登録完了前の発信や発信予約は許可しない
- 設定変更がある場合だけ「変更を適用」を表示して SIP 登録を再開始する。通話中は
  変更適用、待受停止、再接続を許可しない
- Android Service が同一プロセス内の Rust/PJSIP を所有し、UI は状態を監視
- PJSIP の音声 backend は Oboe とし、端末側は 48 kHz、SIP/RTP は PCMU 8 kHz

### Android Push 待受

- Google Play Services と Firebase Cloud Messaging（FCM）を使用し、アプリを
  閉じている間は SIP 登録や foreground service を常駐させない
- tailnet 内の Push gateway が FCM の高優先度 data-only 起動信号を送信したときだけ、
  短時間の foreground
  service から保存済み設定を使って Asterisk へ登録する
- Push gateway の端末向け API は loopback で待ち受け、Tailscale Serve の HTTPS
  だけで公開する。内部 wake API は別の loopback ポートから公開しない
- Ubuntu 上で SIP ユーザに紐づく10分・一度限りの pairing code を発行する
- gateway は128 bitのランダム端末 IDと256 bitの端末資格情報を発行し、Android は
  資格情報を Keystore で保護する。gateway は資格情報の hash だけを保存する
- 同一 SIP ユーザの再ペアリングは以前の端末対応を置き換える
- 起動信号の data は `type=wake` と、暗号学的乱数で生成した 256 bit の
  Base64URL `callToken` だけとする。TTL は 30 秒、collapse は使用しない
- SIP 登録応答は 10 秒待ち、失敗時は自動再試行しない。30 秒以内に着信しなければ
  PJSIP を deREGISTER せず休止して foreground service を終了する
- 登録中は無音・低優先度の「着信接続中」通知を表示する。着信時は Core Telecom
  へ通話を登録し、着信音付き CallStyle 通知へ置き換える。通知から応答、拒否、
  通話終了を操作できる
- Android 14 以上では全画面通知の許可状態と設定導線を表示する。許可済みで画面が
  ロックされている場合は着信画面をロック画面上へ表示し、未許可時は CallStyle
  通知へフォールバックする。サイレントモード等が全画面表示を抑制する場合も
  OS の設定を優先して CallStyle 通知を維持する
- 通話中は Core Telecom の foreground 実行優先度に加え、マイク／通話型 foreground
  service と同じ CallStyle 通知 ID を維持し、
  30 秒を超えても PJSIP と通話を継続する。通話終了後に PJSIP を deREGISTER
  せず休止し、通知と service を終了する
- 通知権限は設定画面の操作時だけ要求する。権限がない場合は Push を処理せず、
  通常の SIP 発着信機能は制限しない
- Firebase Installation ID（FID）は Android Keystore で暗号化し、Android native
  から pairing／更新 API へ直接送る。FID を Rust、Web UI、SIP Contact、
  クリップボード、ログへ出さない
- SIP Contact は FID ではなく gateway が発行したランダム端末 ID を
  `pn-prid` に載せる
- 同一 `callToken` はハッシュだけを保存して2分間重複排除し、token をログへ出さない
- 待受停止中、SIP登録済み、またはSIP登録中の起動信号は既存状態を変えず無視する
- gateway は専用サービスアカウントで FCM HTTP v1 API の `fid` 宛へ送り、
  FID、call token、payload を表示しない
- バッテリー最適化の除外を必須としない
- OpenSIPS は着信 INVITE を保留して Push gateway を起動し、同じ端末 ID の
  再登録後に新しい SIP branch を作る。CANCEL は保留中を含む同じ transaction
  へ中継する

Android の強制停止後の Push 受信は OS の制約により対象外とする。

## 初期対象外

保留、転送、通話待ち、複数通話、複数アカウント、設定プロファイル切り替え、
IPv6、映像、FAX、高音質コーデック、自動更新、録音、Google Play 配布、
Google Play Services を持たない Android 端末。
