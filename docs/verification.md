# Desktop 仕様対応と動作確認

対象は [desktop.md](desktop.md) の MVP。確認日は 2026-09-24。
Windows の GPUI アプリ実体をローカルのモック API で起動し、画面操作と API の再取得で確認しました。
この記録は作業ツリーの検証結果です。本番 Backend、署名済み配布物、macOS / Linux の実機を認証したものではありません。
`docs/task.md` の Backend 実装と Phase 2 は今回の実装対象に含めません。

## 実機モックで確認済みの操作

一覧とパレットは gpui-kit `List` / `ListItem`、`Command` / `Dialog` を使用しています。
以下はこの UI に対する確認結果です。

| 対象 | 確認した操作と結果 |
|---|---|
| My Tasks | 期限切れ / 今日 / 今後 / 期限なし / 完了 のグループに分かれる |
| Task 一覧・詳細 | 行クリックと Down キーで選択を変更し、選択行に対応する詳細が表示される |
| Task 作成・編集 | 作成後に Status / Priority / Assignee / Due Date を変更でき、API の保存値も更新される |
| Comment | Markdown コメントを投稿し、詳細表示と API への保存を確認 |
| Quick Search | Ctrl+P で開き、日本語の「通知」で検索した結果から Task 詳細へ遷移できる |
| Notification Center | Review フィルタと Review / Finding への直接遷移が機能する |
| 一括既読 | Mark all read 後、Header / Sidebar の未読数と API の `unread_count` がすべて 0 になり、Unread 一覧が空になる |
| 存在しない通知対象 | 削除済み Task への通知から詳細を開くと、対象が存在しない旨が表示される |
| テナントをまたぐ通知遷移 | Other Tenant へ切り替えてその Task を表示した後、Mock Tenant の通知をクリックすると、テナントと Review の表示先が自動で切り替わる |
| Finding 遷移 | Fixed → Verified 後、履歴が更新される。PR の unresolved が 4 → 3、blocking が 2 → 1 となり、API の集計と gate 表示が一致する |
| Review 作成 | 不正な Head SHA を拒否する。40 桁の正しい SHA と指摘ゼロで Round を作成でき、R3 タブに No findings が表示される |
| 複数 Finding の作成 | Draft を 2 件追加して一括送信し、同じ Review に保存された本文と件数を API で確認。Finding 本文の Markdown 表示も確認 |
| 通信断 | モックサーバー停止時に Toast と Header の Reconnecting を表示し、アプリの操作を続けられる |
| 通信復旧 | サーバー再起動後に再取得が成功し、接続状態が緑の表示へ戻る |

## UI の表示確認

| 条件 | 確認内容 |
|---|---|
| クライアント領域 960 × 640 | 一覧・詳細・主要操作の表示と、狭いウィンドウでのレイアウトを確認 |
| 最大化したクライアント領域 1920 × 1000 | 一覧・詳細の配置と、ウィンドウ拡大後の表示を確認 |
| Light / Dark | 両テーマで画面を確認 |
| Settings | 縦スクロールで下部の設定まで到達できることを確認 |
| キーボード操作 | kit List の選択と詳細の一致、kit Dialog 内の検索入力と結果選択を確認。Task 入力にフォーカスした状態から Notifications へ移動した後も Ctrl+K が機能する |
| 分割幅の変更 | Review 一覧と詳細の境界をドラッグし、両ペインの幅が変わることを確認 |

Sidebar、Resizable、List、ListItem、TabBar、RadioGroup、Command、Dialog、Markdown は
gpui-kit の部品を使用し、Koyori 固有の行内容と操作を組み合わせています。
通信障害の表示は接続先 URL 全文を出さない短い文言にし、エラー原因は API エラーの内部に保持しています。

## MVP の実装対応と残る確認

「実装対応」はコード上の対応です。実機確認済みの範囲は上表に記載し、未検証の経路は以下に残します。

| 仕様 | 実装対応 | 残る確認・制約 |
|---|---|---|
| §3・§5 技術構成 | Cargo workspace、固定版 gpui-kit 経由の GPUI、非同期 HTTP | macOS / Linux のネイティブビルドと実機確認 |
| §6 認証 | PKCE / state / loopback callback、5 分期限、Credential store、ログアウト、共通 401 検出 | 401 の実機画面遷移は未検証。本番 Device Token フロー、実 Credential store を使う認証・失効・削除も未検証 |
| §7 通知同期 | 30 秒ポーリング、after catch-up、全ページ取得、カーソル保存、初回の過去分 OS 通知抑止 | スリープ復帰・長時間切断・大量通知の実環境確認。ページ途中の失敗、空の初回同期後の複数ページ、重複カーソルは自動テスト対象 |
| §8・§9 常駐・Tray | Open / Notifications / Quit、未読アイコン、Windows の非表示・復帰、macOS Dock badge、Linux tray host 判定 | 背景常駐・Tray 操作・ログイン時起動は実機未検証。Linux は GPUI の hide() 制約により最小化へ fallback |
| §10 OS 通知 | ローカル設定による種別制御、Finding 本文を含めない文言、クリック callback、Windows の MSIX AUMID 取得 | 登録・署名済み配布物での toast 表示とクリックは未検証 |
| §11・§12 通知画面 | 既読・一括既読、Task / Review / Unread フィルタ、履歴ページング、未読数、内部遷移、取得世代管理 | 履歴の追加読込と全フィルタの組み合わせは実機未検証 |
| §14 メイン画面 | My Tasks / Projects、テナント切替、Sidebar / Resizable、分割幅保存 | 分割幅の再起動復元は未検証 |
| §15 Tasks | 一覧・詳細・作成・編集、Status / Priority / Assignee / Due、コメント、完了ステータスに基づく完了操作 | 全フィールドの失敗時ロールバック、全キーボード経路は実機未検証 |
| §16 Reviews | PR / Round / Finding、履歴、Backend の available_actions / gate を表示、Deferred Task リンク、Draft 一括作成、SHA 検証、repo / host 指定 | 追加済み Draft の編集、他の遷移、Deferred Task リンク、403 / 409 の画面操作は未検証。repo / host の本番 API 契約確認が必要 |
| §17 Markdown | Task description / Comment / Review summary / Finding body に kit Markdown を使用 | 全表示箇所での長文・コードブロック・折り返しの網羅確認は未実施 |
| §18・§19 パレット・検索 | kit Command / Dialog、文脈に応じた操作、選択テナント内の Projects / Tasks 検索 | すべての Command、Escape とフォーカス復元の組み合わせは未検証 |
| §21 Theme | Light / Dark / System、semantic token、選択色と通知印のコントラスト調整 | OS の System theme 変更への追従は未検証 |
| §22 Settings | 通知設定、背景常駐、ログイン時起動、Theme、キー設定、Account / Devices / Logout、縦スクロール | キー設定は再起動後適用。Web 通知設定リンクは提供先未実装のため無効。端末名が重複すると現在端末と判定しない。厳密な識別には Backend の明示的な識別情報が必要 |
| §23 エラー処理 | Toast、接続状態と再試行、401 時のセッション破棄、API message、404 表示 | 通信断・復旧・404 は実機確認済み。401 / 403 / 409 の実機画面操作は未検証 |
| §4・§25 3 OS / 配布 | Windows / macOS / Ubuntu CI matrix、MSIX / appinstaller、DMG、AppImage / tar.gz の梱包・署名スクリプト | 署名・notarization・インストール・更新・アンインストールは未実行。macOS / Linux の実機 UI は未検証 |

期限切れトークンによる追加起動は、自動承認審査で「ポリシーによりブロック」として拒否されたため、401 の実機画面確認は実施していません。

## 外部機能と後続タスク

- Device Token と Desktop 向け拡張 API は本番未デプロイです。モックでの成功は本番接続の保証にはなりません。
- Web の通知設定ページは未実装です。隣接する `task` リポジトリの Account / Project settings にも該当ページがなく、存在しない `/settings/notifications` は開きません。
- Device Token、通知拡張、現在端末の識別、Review の repo / host 契約など、Backend 側の対応は TaskCLI の **TASKDESKTO-21** で管理します。
- SSE、Quick Add、Due Date 通知、review.outdated 通知、Review 横断検索、SQLite キャッシュ、macOS / Linux 自動更新は Phase 2 です。

## 再現用モック起動

リポジトリルートの PowerShell から [run-mock.ps1](../desktop/scripts/run-mock.ps1) を実行します。

```powershell
.\desktop\scripts\run-mock.ps1
# ビルド済みの場合
.\desktop\scripts\run-mock.ps1 -NoBuild
# 待受ポートとテスト設定を指定する場合
.\desktop\scripts\run-mock.ps1 -Port 4210 -SettingsPath "$PWD\desktop\target\mock-session\case-2.json"
```

既定 API は `http://127.0.0.1:4209/api`。`KOYORI_DEV_TOKEN` と `KOYORI_SETTINGS_PATH` により、
通常の認証情報と設定ファイルを分離します。実行ファイルもビルド出力からコピーするため、操作中のアプリが再ビルドを妨げません。
ログは `desktop/target/mock-session/` の `app.*.log` / `mock.*.log` に記録します。
アプリ終了後はスクリプトが起動したモックサーバーを停止し、呼び出し元の環境変数を復元します。
背景常駐を有効にした場合は Tray の Quit Koyori で終了してください。
モックの Task / Comment / Notification / Finding の変更はメモリ上にあり、サーバー再起動で初期状態へ戻ります。

## 自動確認

| 確認 | 結果 |
|---|---|
| Workspace テスト | 33 件成功。API / core / mock-api / feature の回帰テストを含む |
| Rust formatting / Clippy | 最終コードで `cargo fmt --all -- --check` と `cargo clippy --locked --workspace --all-targets -- -D warnings` が成功 |
| Windows アプリとモック | ビルドして起動し、上表の操作を確認 |
| 梱包テンプレート | XML エスケープ、MSIX と更新フィードの整合性、plist、バージョン検証、素材存在の自己検証に成功 |
| スクリプト構文 | Python / PowerShell の構文確認済み |

再実行するコマンド:

```powershell
cd desktop
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace
python packaging/package.py --self-test
```

署名情報・OS ごとの前提・配布物の確認方法は [releasing.md](releasing.md) を参照してください。
