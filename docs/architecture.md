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
- シェルや外部 URL を開くプラグインは使用しない。デスクトップの着信音選択だけは
  Rust側のネイティブファイル選択を使い、対応形式を検証してアプリ専用領域へコピーする
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
各端末には専用の SIP ユーザ ID を割り当て、同じ AoR を複数の SIP クライアントで
同時に使用しません。OpenSIPS の複数 Contact による着信先の競合を避けるためです。

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

## デスクトップの着信通知

OS通知には依存せず、Rust側の着信監視が `PhoneSnapshot` を100ms間隔で確認します。
着信中への遷移を一度だけ検出し、通常モードではウィンドウの表示、最小化解除、
フォーカス要求を行います。フォーカスが得られるまでタスクバー／Dockの注意要求を
併用し、着信終了時またはフォーカス取得時に解除します。ウィンドウの最前面固定や、
着信終了後の自動非表示は行いません。

通知モードはRust側でアプリ設定ディレクトリのJSONへ保存し、設定画面とチェック付き
トレイメニューから同じ値を更新します。着信中に通常モードから通知しないモードへ
切り替えた場合は音と注意要求だけを停止し、既に表示したウィンドウは隠しません。
逆方向の切替では、その時点からウィンドウを表示して着信音を先頭から再生します。

着信音の再生とデコードは専用スレッドが所有します。既定音はコード内で生成する
オリジナルの中音域二音チャイムで、2回のチャイムと約2秒の無音を一周期とします。
取り込んだWAV、MP3、OGG Vorbis、FLACはアプリデータディレクトリへコピーし、
全体をメモリへ読み込まずループデコードします。選択音を開けない場合は既定音へ、
選択出力を開けない場合はOS既定出力へ、その再生だけフォールバックします。
再生中の出力切断は音声ストリームのエラーを監視し、OS既定出力で再開します。

試聴と実着信は同じ再生経路を使います。実着信を優先し、試聴は10秒、画面遷移、
通話開始、明示停止、アプリ終了のいずれかで終了します。音量0～100はプレイヤーの
減衰率へ対応し、100でも元音源を増幅しません。呼の状態機械は着信音の失敗から
独立しており、音声出力が利用できなくても応答・拒否できます。

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

### Push gateway と端末ペアリング

アプリ終了中の着信では、tailnet 内で常時稼働する Push gateway が FCM の
起動信号だけを Android へ送ります。SIP 信号と音声は FCM や gateway を通りません。

```text
OpenSIPS -- device ID --> Push gateway -- FID --> FCM
                                              |
                                              | data-only: type=wake, callToken
                                              v
                                     FirebaseMessagingService
                                              |
                                              v
                                      TelephonyService
                                              |
                                              +-- Tailscale/SIP --> OpenSIPS/Asterisk
```

Push gateway は Ubuntu サーバーの loopback だけで待ち受けます。端末から使う
pairing API と FID 更新 API のみを Tailscale Serve の HTTPS で tailnet へ公開し、
OpenSIPS が使う wake API は別ポートの loopback に留めます。

Ubuntu 上で SIP ユーザに紐づく10分・一度限りの pairing code を発行します。
Android native は pairing code と FID を HTTPS で gateway へ送り、gateway が生成した
128 bit のランダムな端末 ID と 256 bit の端末資格情報を受け取ります。端末資格情報は
Android Keystore で保護し、gateway は SHA-256 hash だけを SQLite に保存します。
同じ SIP ユーザを再ペアリングした場合は以前の端末対応を置き換えます。

FID は Android native と Push gateway の境界内だけで扱い、Rust、Web UI、SIP、
Contact、ログには出しません。SQLite は gateway 実行ユーザだけが読める権限にします。
SIP REGISTER の Contact URI には FID ではなく次の情報を載せます。

```text
;pn-provider=fcm;pn-prid=<random device ID>;pn-param=app.tsumujiphone.local
```

Firebase SDK が FID を更新した場合、Android native は保存済み端末資格情報を使って
gateway の対応表だけを更新します。

### FCM 起動処理

```text
Push gateway
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
    +-- no INVITE: dormant stop by 30 s
    |
    +-- INVITE: Core Telecom + CallStyle -> call -> dormant stop
```

FCM SDK は registration token ではなく FID の直接送信 API を使用します。
`onRegistered(installationId)` で受け取った FID は専用の Android Keystore 鍵で
暗号化します。

FCM callback は高優先度、data key の完全一致、通知権限、待受意図、call token の
形式と重複を検証します。call token の生値は Service へ渡さず、SHA-256 hash だけを
2分保持します。登録済み／登録中なら foreground service を開始しません。

有効な wake は `shortService` 型 foreground service を起動し、即時に
「着信接続中」を表示します。Tauri Activity を作らないプロセス起動にも対応するため、
Android Service と Tauri UI は遅延初期化した同じ Rust/PJSIP engine を共有します。
登録応答は10秒待ちます。30秒以内に着信しなければ PJSIP は deREGISTER を送らず
休止し、OpenSIPS 上の Push 対応 Contact を維持します。

OpenSIPS は Asterisk からの着信 INVITE を保留し、Contact のランダム端末 ID を使って
Push gateway を呼びます。端末が再登録すると `mid_registrar` が保留中 INVITE に
新しい branch を作ります。端末の SIP stack は待受窓と通話の終了時に deREGISTER
しないため、`pn_skip_pn_interval=0` とし、連続着信でも毎回 Push と再登録を行います。
CANCEL は同じ transaction に中継します。

## Android プラットフォーム層

Android Service が同一プロセス内の Rust/PJSIP engine を所有し、Tauri UI は
service の状態を監視します。ユーザー向けの待受状態は「待受停止」「接続中」
「待受中」「接続失敗」の四つに統一します。通話の有無と段階は待受状態とは別に
`PhoneSnapshot` で表します。

SIP パスワードと Push gateway の端末資格情報は別々の Android Keystore 鍵で保護し、
保存先は各一件だけとします。FID も専用の鍵で保護します。ローカル SIP アドレスは
Tailscale IPv4 から自動決定し、設定画面へ入力欄を出しません。明示的な待受停止では
自動開始の意図を解除しますが、保存済み SIP サーバー、ユーザ ID、パスワードは維持
します。FID は待受停止時も維持します。

PJSIP は Oboe を使用し、端末音声48 kHz と PCMU/RTP 8 kHz の変換を担当します。
バッテリー最適化の除外は要求しません。発着信は Core Telecom へ登録し、同じ
CallStyle 通知 ID を通話終了まで更新します。着信通知は着信音、全画面 Intent、
応答・拒否操作を持ち、通話中通知は通話終了操作を持ちます。Core Telecom の
foreground 実行優先度に加え、応答後はマイク／通話型 foreground service として
同じ通知を維持します。これにより Activity を前面へ出さず、30秒を超える通話でも
process とバックグラウンド音声を維持します。

通話中に Android のタスク一覧から Activity が削除されると、Tauri の
`RunEvent::ExitRequested` で通常終了を一時保留します。Rust/PJSIP engine へ切断を
要求し、Service が native call の終了を検出して Core Telecom と CallStyle 通知を
終了する処理に1秒の猶予を与えてから、プロセスを終了します。通話がない場合は
終了を保留しません。これは OS 設定から行う「強制停止」とは区別します。

通話音声の経路は `AudioManager` で直接変更せず、通話ごとの
`CallControlScope.currentCallEndpoint` と `availableEndpoints` を Service が監視し、
`requestEndpointChange` で切り替えます。Tauri UI は Service が保持する現在経路と
候補を通話中だけ監視し、Activity の再表示時も実状態を取得し直します。通話終了時は
経路状態を破棄し、次の通話へ選択を引き継ぎません。

Android 12 以上の `BLUETOOTH_CONNECT` は、ユーザーが初めて通話画面の音声経路一覧を
開いたときだけ要求します。拒否時は Bluetooth の候補を除外して受話口、スピーカー、
有線経路を維持し、再許可はアプリ詳細設定から行います。Bluetooth 機器名は経路選択
画面だけへ返し、Rust、永続ストア、ログ、Push gateway へ渡しません。経路準備と変更
要求は5秒で打ち切り、自動再試行せず Android が選んだ現在経路を維持します。

端末再起動後もアプリを開く前に SIP 経路を利用できるよう、Tailscale は Android の
「常時接続 VPN」として自動起動させます。VPN 外の通信をブロックする設定は
アーキテクチャ上の必須条件としません。

Android 14 以上では `NotificationManager.canUseFullScreenIntent()` を設定画面へ
反映し、未許可時は `ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT` を開きます。許可時は
`showWhenLocked` と `turnScreenOn` を指定した既存 Activity を着信画面として使い、
未許可時またはサイレントモード等による視覚抑制時は CallStyle 通知だけで処理します。
Push 通話から起動した Activity が通話終了後も残る場合は、短時間登録を休止して
started service 状態を解除した後、bound service として通常の待受登録へ移行します。

## 配布対象

- 最小 API: 31（Android 12）
- 実機 ABI: `arm64-v8a`
- エミュレーター ABI: `x86_64`
- 初期配布: 直接配布する署名済み APK
- push: Google Play Services の FCM のみ
