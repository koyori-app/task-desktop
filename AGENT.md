## 概要

https://github.com/koyori-app/task
のデスクトップアプリ。詳細な仕様は docs/desktop.mdを参照。

## コーディング規則

docs配下の仕様書を読むこと。その際task.mdは現在実装中であるためdesktop.mdの内容を実装する際に実装しないこと。それをTaskCLIでタスク化して後で再開できるようにすることが求められる。

## Git

- コミットメッセージに `Co-Authored-By` トレーラーや `Generated with …` 等の
  プロモーション行を付けない。本文は変更内容だけを書く
- GPG 署名は不要（`git -c commit.gpgsign=false commit`）
- 適宜コミットして進める。日本語の Conventional Commits

## 環境

- Cargo ワークスペースは `desktop/`。ビルド: `cargo build -p app`、実行: `cargo run -p app`（`target/debug/koyori.exe`）
- Rust は scoop の `rustup`（`CARGO_HOME`/`RUSTUP_HOME` = `~\scoop\persist\rustup\.cargo|.rustup`、`.cargo\bin` が User PATH 済み）。MSVC は VS Build Tools 2022 + VCTools
- エージェントのシェルは環境変数が古いままのことがある。コマンド先頭で
  `$env:PATH = [Environment]::GetEnvironmentVariable('PATH','Machine') + ';' + [Environment]::GetEnvironmentVariable('PATH','User')`
  を入れてから cargo を呼ぶと確実
- taskCLI: `task` コマンド（`~\.local\bin`、config は `~\.config\task\config.yaml`）。api_url=https://task.koyori.app/api
- Windows Defender のリアルタイム保護が target/ の .o 削除をロックして `os error 32` で稀にビルド失敗する。除外設定は要管理者権限

## モックサーバー（ログイン前の動作確認用）

Device Token フローが本番未デプロイの間は `crates/mock-api`（bin `koyori-mock`）で
spec 準拠のモックを立てて動作確認できる。

```powershell
cargo run -p mock-api          # http://127.0.0.1:4199/api で待受（KOYORI_MOCK_PORT で変更可）
$env:KOYORI_API_BASE='http://127.0.0.1:4199/api'; $env:KOYORI_DEV_TOKEN='dev'
& .\target\debug\koyori.exe    # credential store を bypass してログイン済み状態で起動
```

- `KOYORI_DEV_TOKEN` があると main.rs が credential store ではなくその値で Client を作る
- タスク/コメント/通知既読/Finding 状態変更はモックのメモリ上で反映される（再起動でリセット）
- モック側の stderr に全リクエストがログされるので、アプリが何を叩いているか追える