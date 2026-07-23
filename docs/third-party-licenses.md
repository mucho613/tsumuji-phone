# サードパーティライセンス

リリース前に `cargo deny` と npm のライセンス一覧を実行し、正確なバージョンと
ライセンス本文を配布物へ含めます。

主要な直接依存:

- PJSIP 2.17: GPL-2.0-or-later（GPL 選択で利用）
- Tauri 2: Apache-2.0 / MIT
- React: MIT
- Lucide: ISC

Hikari Phone 本体は GPL-3.0-or-later です。PJSIP の固定ソースは
`vendor/pjproject` Git サブモジュールとして管理します。
