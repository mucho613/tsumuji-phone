# Hikari Phone

RT-500KI の「ひかり電話」を同一 LAN 内から利用する、個人向けの
デスクトップ／Android クライアントです。現在のデスクトップ実装は Ubuntu
24.04 と Windows 11 x64 を対象に、Rust、Tauri 2、React、PJSIP 2.17 で
構成します。

> 現在は初期開発段階です。緊急通報（110、118、119）には対応せず、アプリ側でも
> 発信を拒否します。既存の電話機を緊急時に利用できる状態で維持してください。

## 現在の構成

- `crates/telephony-core`: PJSIP に依存しない状態機械とエンジン契約
- `crates/telephony-pjsip`: 薄い C ABI と Rust FFI
- `crates/hikari-phone-cli`: バックエンド診断 CLI
- `native/pjsip-adapter`: PJSUA-LIB を扱う C アダプター
- `src-tauri`: デスクトップアプリの Rust バックエンド
- `src`: React UI
- `vendor/pjproject`: PJSIP 2.17 の固定サブモジュール

## 開発

必要な開発ツールを確認します。このスクリプトはパッケージをインストールしません。

```bash
./scripts/check-dev-env.sh
```

フロントエンドと Rust の通常テスト:

```bash
npm install
npm run build
cargo test --workspace
```

PJSIP を静的ビルドしてネイティブアダプターを検査:

```bash
./scripts/build-pjsip-linux.sh
source target/pjproject-env.sh
cargo check -p telephony-pjsip --features native-pjsip
cargo run -p hikari-phone-cli -- backend
```

アプリを PJSIP 付きで起動:

```bash
source target/pjproject-env.sh
npm run tauri dev -- --features native-pjsip
```

Windows 11 での環境構築と PJSIP ビルドは
[docs/windows-development.md](docs/windows-development.md) を参照してください。

RT-500KI の設定は [docs/rt500ki-setup.md](docs/rt500ki-setup.md)、
設計上の境界は [docs/architecture.md](docs/architecture.md) を参照してください。

## ライセンス

GPL-3.0-or-later。依存ライブラリの条件は
[docs/third-party-licenses.md](docs/third-party-licenses.md) に記録します。
