## 概要

https://github.com/koyori-app/task
のデスクトップアプリ。詳細な仕様は docs/desktop.mdを参照。

## コーディング規則

docs配下の仕様書を読むこと。その際task.mdは現在実装中であるためdesktop.mdの内容を実装する際に実装しないこと。それをTaskCLIでタスク化して後で再開できるようにすることが求められる。

## 環境

- Cargo ワークスペースは `desktop/`。ビルド: `cargo build -p app`、実行: `cargo run -p app`（`target/debug/koyori.exe`）
- Rust は scoop の `rustup`（`CARGO_HOME`/`RUSTUP_HOME` = `~\scoop\persist\rustup\.cargo|.rustup`、`.cargo\bin` が User PATH 済み）。MSVC は VS Build Tools 2022 + VCTools
- エージェントのシェルは環境変数が古いままのことがある。コマンド先頭で
  `$env:PATH = [Environment]::GetEnvironmentVariable('PATH','Machine') + ';' + [Environment]::GetEnvironmentVariable('PATH','User')`
  を入れてから cargo を呼ぶと確実
- taskCLI: `task` コマンド（`~\.local\bin`、config は `~\.config\task\config.yaml`）。api_url=https://task.koyori.app/api
- Windows Defender のリアルタイム保護が target/ の .o 削除をロックして `os error 32` で稀にビルド失敗する。除外設定は要管理者権限