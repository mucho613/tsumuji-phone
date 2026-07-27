# Firebase 開発手順

## ローカルファイル

Firebase Console で Android アプリ `app.tsumujiphone.local` を登録し、取得した
`google-services.json` を次へ配置します。

```text
src-tauri/gen/android/app/google-services.json
```

このファイルがない Android build は、配置先を示して構成段階で失敗します。
Firebase なしの APK は生成しません。このファイルとサービスアカウント鍵は
`.gitignore` の対象であり、commit しません。

FCM HTTP v1 API 用の専用サービスアカウント鍵は workspace の外へ保存します。
ローカル開発時の例:

```text
C:\Users\mucho\.config\tsumuji-phone\fcm-sender.json
```

## Android build

Android Studio 同梱 JDK と Android SDK/NDK を現在の PowerShell セッションへ設定して
から build します。

```powershell
$env:JAVA_HOME = "C:\Program Files\Android\Android Studio\jbr"
$env:ANDROID_HOME = "$env:LOCALAPPDATA\Android\Sdk"
$env:NDK_HOME = "$env:ANDROID_HOME\ndk\29.0.14206865"
$env:Path = "$env:JAVA_HOME\bin;$env:ANDROID_HOME\platform-tools;$env:Path"
npm run tauri -- android build --debug --apk --target aarch64
```

出力:

```text
src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

## FCM 起動試験

通常の試験では FID をコピーせず、Push gateway とのペアリング後に gateway の
内部 wake API を使います。構築、ペアリング、送信手順は
[`push-gateway.md`](push-gateway.md) に記載します。

1. APKを起動し、通常のSIP設定を保存して「待受中」まで確認する
2. 設定の「Push待受」で「通知を許可」を押す
3. FCMが「準備完了」になったら、gateway URLと一度限りのコードでペアリングする
4. アプリのタスクを閉じる。Androidの「強制停止」は行わない
5. Ubuntuサーバーのloopback-only wake APIへランダム端末IDを送る

端末では無音・低優先度の「着信接続中」通知が表示され、Asteriskへの登録成功後も
FCM受信から30秒まで維持された後、SIP登録とともに消えます。失敗時は
「着信接続に失敗しました」が残り、詳細は次回アプリ起動後の設定画面で確認します。

受入試験では通常のタスク終了に加えて、次も使用できます。

```powershell
adb shell am kill app.tsumujiphone.local
```

画面ロック＋Dozeは、端末をロックした後に次で移行させます。試験後は解除します。

```powershell
adb shell dumpsys deviceidle force-idle
adb shell dumpsys deviceidle unforce
```

## 秘密情報の扱い

- FID、端末資格情報、call token、サービスアカウント鍵、SIP資格情報をログへ出さない
- FIDはAndroid nativeとPush gatewayだけで扱い、Rust/Web UI/SIP/クリップボードを
  通さない
- call tokenは32 byteの暗号学的乱数とし、Androidには重複検出用SHA-256 hashだけを
  2分間保存する
- サービスアカウント鍵を失効・交換した場合は、古い鍵をGoogle Cloud側で削除する
