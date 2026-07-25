# Windows 開発環境

Windows 11 x64 と Visual Studio 2022 の MSVC ツールチェーンを対象とします。
WSL 内ではなく、Windows の PowerShell から実行してください。

## 1. 開発ツール

次をインストールします。

- Git for Windows
- Rustup（既定の `stable-x86_64-pc-windows-msvc`）
- Node.js 22 LTS
- Visual Studio 2022 Build Tools
  - `Desktop development with C++`
  - Windows 11 SDK
  - MSVC v143 x64/x86 build tools
- WebView2 Runtime（通常は Windows 11 に導入済み）

環境を確認します。

```powershell
.\scripts\check-dev-env.ps1
```

## 2. リポジトリ

サブモジュールを含めて取得します。

```powershell
git clone --recurse-submodules <repository-url>
cd hikari-phone
```

既に clone 済みの場合は次を実行します。

```powershell
git pull
git submodule update --init --recursive
```

## 3. 通常のビルド

```powershell
npm ci
npm run build
cargo test --workspace
```

UI だけを確認する場合:

```powershell
npm run dev
```

## 4. PJSIP を含むビルド

PJSIP 2.17 の `Release|x64` 集約静的ライブラリを MSBuild で作成します。
スクリプトはプロジェクト用の `config_site.h` をサブモジュールのビルドツリーへ
コピーします。

```powershell
.\scripts\build-pjsip-windows.ps1
cargo check -p telephony-pjsip --features native-pjsip
cargo run -p tsumuji-phone-cli --features native-pjsip -- backend
```

デスクトップアプリを起動します。

```powershell
npm run tauri dev -- --features native-pjsip
```

NSIS インストーラーを作成する場合:

```powershell
npm run tauri build -- --features native-pjsip
```

## 制約

- Windows は `x86_64-pc-windows-msvc` のみを対象とします。
- PJSIP は `Release|x64`（動的 CRT、静的 PJSIP ライブラリ）に固定します。
- Windows 上の実機通話、音声デバイス列挙、インストーラーは Windows マシンで
  検証する必要があります。
