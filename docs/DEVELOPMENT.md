# 開発ガイド

ソースからの起動・検証・署名付きビルド・更新配信を行う開発者と配布者向けの手順です。アプリを使う方は [README](../README.md) と [操作ガイド](UI_GUIDE.md) を参照してください。

## 開発環境と検証

画面は Tauri 2 + React + TypeScript、通知転送・認証・暗号化は Rust で実装しています。通常の Windows ビルドには Node.js 22 LTS、Rust stable の MSVC ツールチェーン、Visual Studio Build Tools の「C++ によるデスクトップ開発」、WebView2 が必要です。VS Code だけでは MSVC の `link.exe` は導入されません。

```powershell
npm.cmd ci
npm.cmd run desktop:dev
```

```powershell
npm.cmd run build
cargo test --locked
cargo test --manifest-path src-tauri/Cargo.toml --locked --features custom-protocol
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --features custom-protocol -- -D warnings
node --test scripts/release-manifest.test.mjs
```

インストーラーを一度ビルドすると、Tauri の NSIS を使ってアンインストール時の設定削除を検証できます。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-installer-hooks.ps1
```

この検証はワークスペースの `target` 内に作ったダミー設定だけを使います。実際のアプリや AppData は削除せず、設定を残す場合・自動更新時・明示的にデータ削除を選んだ場合を確認します。カスタム保存先の削除処理は `src-tauri/installer-hooks.nsh` にあり、Tauri 標準の削除チェックが選択され、更新モードでない場合だけ動作します。

署名付き配布ビルドには更新用の秘密鍵が必要です。[ビルド・公開手順](#ローカルで署名付きビルドを作る) を参照してください。用意済みの GNU / LLVM-MinGW 環境を使うローカルビルドは `npm.cmd run desktop:build:local` で実行でき、既定では `target/tauri` に出力します。このコマンドは別の PC にコンパイラーを自動インストールするものではありません。

[設定のサンプル](../config.example.json) は参照用です。GUI からの保存後は自動で接続を再開し、設定ファイルを直接編集した場合は再起動が必要です。画面だけを試すには `discord_To_VR.exe --ui-preview` を使います。プレビューはダミー情報で動き、Discord・XSOverlay への接続や設定の読み書きは行いません。

ソース内の旧ネイティブ版では `cargo run -- --no-gui` / `--check-config` を維持しています。一般配布向けの画面は Tauri 版です。

## 配布者の初回設定

Windows の発行元情報は `src-tauri/tauri.conf.json` の `bundle.publisher` で `Discord to VR` に設定しています。アプリ一覧の発行元、実行ファイルとインストーラーの会社名へ反映します。この値と Windows の Authenticode 署名は別のものです。UAC・SmartScreen の認証済み発行元にはコード署名証明書が必要で、更新用の署名鍵ではその表示を変更できません。NSIS は発行元をインストール先の記録にも使うため、公開後は同じ値を維持してください。

この環境では更新用鍵を生成し、公開鍵をアプリへ組み込みました。秘密鍵はリポジトリ外の次の場所に保存しています。

```text
%LOCALAPPDATA%\DiscordToVR-release-signing\updater.key
```

この鍵を安全な場所にバックアップしてください。公開済みアプリへの更新には同じ鍵が必要です。上書き生成やリポジトリへの追加、ZIP への同梱、チャット・Issue への貼り付けはしないでください。

1. GitHub リポジトリの Settings → Secrets and variables → Actions で、`TAURI_SIGNING_PRIVATE_KEY` に秘密鍵ファイルの**内容**を登録します。ファイルのパスではありません。この環境の鍵はパスワードなしで生成したため、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` は未設定で構いません。
2. `src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`package.json` と `package-lock.json` のアプリバージョンを揃えます。ルートの Cargo は共通ライブラリなので変更不要です。
3. 検証後、同じバージョンの安定版タグ（初回は `v0.2.0`、次回は例えば `v0.2.1`）を公開します。タグの Actions が署名付き NSIS、`.sig`、`latest.json`、ZIP を作り、GitHub Release に添付します。`release` ブランチだけのビルドは Actions の成果物を作り、公開更新を変更しません。

公開版の更新配信には上の設定が必要です。別のリポジトリへ移す場合は配布先の固定 URL と検証スクリプト、鍵を自分のものへ変更してください。

## ローカルで署名付きビルドを作る

この PC は MSVC の `link.exe` がないため、用意済みの Rust GNU ツールチェーンと LLVM-MinGW を使います。プロジェクトのルートで次のコマンドを実行してください。

```powershell
npm.cmd run desktop:build:local
```

このコマンドは `%TEMP%\discord-vr-portable-compiler` の msvcrt / x86_64 コンパイラーと `gcc-runtime` を確認して、ビルド用プロセス内だけに環境変数を設定します。既定の Rust ツールチェーン・システム PATH・PowerShell の実行ポリシーは変更しません。署名用鍵は上記の既存ファイルを使い、再生成しません。`CARGO_TARGET_DIR` の指定がなければ `target/tauri` へ出力し、成功後に `latest.json` も生成します。コンパイラーを別の場所へ移した場合は `npm.cmd run desktop:build:local -- -CompilerRoot 'D:/path/to/discord-vr-portable-compiler'` と指定します。

別の PC で通常の MSVC ビルドをする場合は、Visual Studio Build Tools の「C++ によるデスクトップ開発」（MSVC と Windows SDK）をインストールし、Developer PowerShell などで `link.exe` が利用できる環境から次を実行します。VS Code のみではリンカーは導入されません。GitHub の Windows Actions は MSVC が利用できる環境なので通常のコマンドを使います。

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Join-Path $env:LOCALAPPDATA 'DiscordToVR-release-signing/updater.key'
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
$env:CARGO_TARGET_DIR = Join-Path (Get-Location) 'target/tauri'
npm.cmd run desktop:build -- --ci -- --locked
node scripts/release-manifest.mjs
```

`latest.json` はアプリバージョン・タグ・インストーラー名・署名のバージョンが一致する場合だけ生成します。公開する Release にインストーラーと `.sig` と `latest.json` を添付してください。秘密鍵を紛失した場合、既存の公開鍵を持つアプリへ新しい鍵で自動更新することはできません。

仕組みの詳細: [Tauri 2 の Updater](https://v2.tauri.app/plugin/updater/)
