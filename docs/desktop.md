# Koyori Desktop 仕様書

## 1. 概要

### 1.1 プロダクト名

Koyori Desktop

### 1.2 目的

Koyori の Task・Review のイベントを受信し、OS ネイティブ通知から確認・対応できる
デスクトップクライアント。Backend 側の対応は task.txt（Desktop Client 対応仕様書）。

Web 版の移植ではなく、以下を Desktop の価値とする。

- OS ネイティブ通知と、通知から対象への直接遷移
- バックグラウンド常駐
- Review への迅速な対応
- Keyboard-first の操作

### 1.3 フェーズ

| フェーズ | 内容 |
|---|---|
| MVP | 認証、通知（ポーリング + catch-up）、OS 通知、Notification Center、Task 一覧/詳細/作成/編集、Review 一覧/詳細/遷移/作成、Command Palette、Settings |
| Phase 2 | Realtime（SSE）、Quick Add、Due Date 通知、`review.outdated`、Quick Search の Review 横断検索、Auto Update の差分配信、SQLite キャッシュ |

---

## 2. 設計原則

| 原則 | 内容 |
|---|---|
| Notification First | 画面を見ていなくても自分に関係する変更を認識できる |
| Actionable | 通知クリックで Task / Review / Finding へ直接移動する |
| Native | WebView を使わず GPUI で描く |
| Reliable Delivery | Realtime だけに依存しない。停止・切断中の通知は再接続後に `after` カーソルで取り切る |
| API Driven | Backend が正。**遷移規則・マージ可否を Desktop で再計算しない**（`available_actions` / `gate` を使う） |
| Keyboard First | 主要機能をキーボードだけで操作できる |

---

## 3. 技術構成

| 項目 | 技術 |
|---|---|
| Language | Rust（stable） |
| GUI | GPUI（gpui-kit が固定するスナップショット版） |
| UI | gpui-kit |
| API | Koyori API（OpenAPI から型生成） |
| HTTP | reqwest |
| Realtime（Phase 2） | SSE |
| Credential | OS credential store（`keyring`） |
| Local Cache（Phase 2） | SQLite |

- **`gpui` を直接依存に書かない**。gpui-kit が `gpui-pre` を完全固定（`=x.y.z`）で
  再エクスポートしているので、それだけを使う。別に `gpui` を足すと 2 系統の GPUI が
  混ざってビルドが壊れる
- gpui-kit は pre-1.0 で破壊的変更が頻繁。バージョンは Cargo.toml で完全固定し、
  上げるときは 1 PR で全 crate をまとめて追従する
- UI は gpui-kit の Component / Primitive を使う。Koyori 固有の UI だけ組み合わせで作る

---

## 4. 対応 OS

初期対応: **Windows / macOS / Linux**（3 OS とも MVP に含める）。

GPUI は Windows を追加 feature 無しで対応する（Win32 + DirectWrite、描画は DirectX）。
gpui-kit も 3 OS を対象にしている。OS 連携は gpui-kit の範囲外なので Platform 層（§5）で吸収する。

### 4.1 Windows

| 項目 | 方針 |
|---|---|
| 配布 | **MSIX**。AUMID・Start Menu 登録・自動更新が付く。toast 通知は AUMID が無いと表示されない |
| 署名 | コード署名必須（SmartScreen）。§37 |
| toast クリック | **常駐中のみ受ける**。未起動時のクリック（COM Activator 登録）は対応しない。常駐前提（§8）なので実害が無く、COM 登録を避けられる |
| ビルド | MSVC（Visual Studio Build Tools「C++ によるデスクトップ開発」）。mold は使わない |
| Tray | `tray-icon`。未読はアイコン差し替えで表現 |
| CI | **Windows ランナーを初日から入れる**。Linux だけで回すと Windows 固有のコンパイルエラーに気づかない。GPUI のビルドが重いので target ディレクトリのキャッシュを先に設計する |

### 4.2 macOS

Tray = メニューバー、未読は Dock badge。署名と notarization が必要。

### 4.3 Linux

Tray は環境依存（GNOME は拡張なしで無い）。Tray が無い環境では Main Window を閉じたら
終了する（§8）。通知は `notify-rust`（D-Bus）。配布は AppImage と tar.gz。

---

## 5. アプリケーション構成

Cargo Workspace。

```text
desktop/
├── Cargo.toml
├── rust-toolchain.toml
├── crates/
│   ├── app/         起動・Window・Tray・Platform 層の結線
│   ├── core/        Application State / 通知同期 / 設定 / Credential
│   ├── api/         OpenAPI 生成型 + reqwest クライアント
│   ├── platform/    OS 連携（下表）
│   └── features/    notifications / reviews / tasks / projects / search / settings
└── assets/
```

Platform 層は自作せず既存 crate を薄く包む。

| 機能 | crate |
|---|---|
| Tray | `tray-icon` |
| OS 通知 | `notify-rust`（Windows は WinRT toast 経由） |
| グローバルショートカット | `global-hotkey` |
| ログイン時起動 | `auto-launch` |
| Credential | `keyring`（Windows Credential Manager / macOS Keychain / Linux Secret Service） |

構造:

```text
GPUI / gpui-kit → Feature → Application State → API Client → Koyori Backend
```

- View から直接 HTTP を呼ばない
- API 処理で UI スレッドを塞がない
- 共通 UI crate を先に作らない。共通化の価値が出たものだけ昇格させる

---

## 6. 認証

Backend の Device Token（task.txt §17）を使う。PAT は使わない（テナント束縛で通知 API を叩けない）。

```text
Desktop: 127.0.0.1:{port} で待ち受け、code_verifier / state を生成
   │
   ▼ システムブラウザで開く
{web}/desktop/authorize?port=&code_challenge=&state=&name={端末名}
   │  （未ログインなら通常ログイン。2FA もここで済む）
   ▼ 承認
http://127.0.0.1:{port}/callback?code=&state=
   │
   ▼ state 検証 → POST /v1/desktop/auth/token {code, code_verifier}
Device Token → OS credential store
```

- **Custom URI Scheme は使わない**（loopback のみ）。Linux で登録不要、Windows で
  レジストリ書き込み不要、他アプリに奪われない、ファイアウォールの警告も出ない
- Token は credential store 以外に置かない。設定ファイル・ログに出さない
- Token は 90 日で失効。401 を受けたら未ログイン状態に戻し、再承認を促す
- Logout = `DELETE /v1/users/me/devices/{self}` + credential store から削除
- ブラウザ承認の待ち受けは 5 分でタイムアウトし、port を閉じる

---

## 7. 通知の取得

### 7.1 MVP: ポーリング + catch-up

```text
起動 / 復帰
   │
   ▼ GET /v1/users/me/notifications?after={last_cursor}   （初回は ?limit=50）
   │   next_cursor がある間は続きを取る
   ▼
last_cursor を更新 → 新着を OS 通知 + Notification Center へ
   │
   └─ 30 秒ごとに繰り返す
```

- `last_cursor` は最新行の `(created_at, id)`。ローカル設定に保存する
- `unread=true` を catch-up に使わない（既読だが未受信の通知を落とす）
- 初回起動（カーソル無し）の未読は OS 通知を**出さない**（起動直後に過去分が一斉に鳴らない）
- `unread_count` は一覧レスポンスの値を使う（専用エンドポイントは無い）

### 7.2 Phase 2: Realtime（SSE）

`GET /v1/users/me/notifications/stream` を `Last-Event-ID = last_cursor` で接続する。
切断は指数バックオフで再接続し、再接続時も §7.1 の catch-up を必ず通す。
接続中もポーリングを 5 分間隔のフォールバックとして残す。

---

## 8. バックグラウンド常駐

設定「Keep Running in Background」が有効なら、Main Window を閉じてもプロセスを終了しない。

```text
Window Close → Hide → Tray 常駐（ポーリング / SSE 継続、OS 通知表示）
```

明示的な `Quit Koyori` で終了する。Tray が無い環境（Linux の一部）ではこの設定を無効化し、
閉じたら終了する。

---

## 9. System Tray

```text
Open Koyori
Notifications
Quick Add         （Phase 2）
────────────
Quit Koyori
```

未読があれば Windows / macOS はアイコン（Dock badge）へ反映する。

---

## 10. OS 通知

新着 1 件につき 1 通。文言は `notification_type` + `payload` + target の解決結果から
クライアントが組み立てる（Backend は文言を返さない）。

```text
Koyori
Review finding fixed
PR #742 · R2 · yupix
```

- Finding の title / body は OS 通知に出さない（機密になり得る。詳細はクリック後に
  対象 API から取る）
- 同一操作で複数件届いた場合（例: 一括作成）でも Backend が 1 件に畳んでいる
  （`review.round_created` に件数）ので、Desktop 側で束ねない
- 「Enable Desktop Notifications」と種別（Task / Review / Due）の ON/OFF は
  **Desktop のローカル設定**。OFF でも Notification Center には出る。Backend の
  `notification_settings`（プロジェクト単位・保存するか否か）とは別物

---

## 11. 通知からの Navigation

通知は `target` を持つ。Desktop が内部ルートへ変換する。

| `target.type` | 遷移先 |
|---|---|
| `task` | Project → Task 詳細 |
| `review` | Project → Reviews → PR → ラウンド |
| `review_finding` | Project → Reviews → PR → Finding 詳細（`deferred_task_id` があれば Task へのリンクも出す） |

- `project`（tenant_id / project_id / key）は通知レスポンスの要約から取る
- クリックで Main Window を表示し、起動済みなら Foreground へ
- 対象 API が 404 なら「This task no longer exists.」等を表示し、通知は既読にする

---

## 12. Notification Center

```text
Notifications                      [Mark all read]

● Review   Finding fixed · PR #742          2 minutes ago
● Task     TASK-482 assigned to you        18 minutes ago
○ Review   Finding verified · PR #738      Yesterday
```

- 未読 / 既読、すべて既読（`PATCH …/read-all`）
- `kind=task|review` による絞り込み、`unread=true`
- 並びは新着順のみ（Backend も新着順のみ）。未読を上に出す場合は `unread=true` で別に引く
- 下端で `cursor` を使って続きを読む
- 新着はポーリング / SSE の結果を先頭へ挿入
- Header の 🔔 に `unread_count`

---

## 13. 通知対象

Backend の種別をそのまま使う（task.txt §5）。

| kind | 種別 | 状態 |
|---|---|---|
| Task | `assigned` / `mentioned` / `status_changed` / `comment_added` | MVP |
| Task | `deadline_soon` | Phase 2 |
| Review | `review.round_created`（件数・Blocking 件数込み） / `review.finding_fixed` / `review.finding_reopened` / `review.finding_verified` / `review.finding_deferred` | MVP |
| Review | `review.outdated` | Phase 2 |

新しい Finding 単体・Blocking 単体の通知は無い（ラウンド通知に含まれる）。

---

## 14. メインウィンドウ

```text
┌──────────────────────────────────────────────────────┐
│ Koyori                              Search   🔔   ⚙  │
├─────────────┬─────────────────────┬──────────────────┤
│ Sidebar     │ Content             │ Detail           │
│             │                     │                  │
│ My Tasks    │                     │                  │
│             │                     │                  │
│ Projects    │                     │                  │
│             │                     │                  │
│ Notifications                     │                  │
└─────────────┴─────────────────────┴──────────────────┘
```

- **Inbox は置かない**（Koyori に対応する概念が無い）。My Tasks は `/v1/users/me/tasks`。
  Today / Upcoming は別画面にせず、My Tasks の中を期限で
  「期限切れ / 今日 / 今後 / 期限なし / 完了」に分けて表示する
- テナントは Header で切り替える。Sidebar の Projects は選択中テナントのもの
- gpui-kit の Dock / Resizable を使い、レイアウト状態はローカルに保存する

---

## 15. Tasks

MVP:

- 一覧（プロジェクト / My Tasks）、詳細、作成、編集
- Status / Priority / Assignee / Due Date の変更
- **コメントの表示と投稿**（`mentioned` / `comment_added` の飛び先で読めないと通知の意味が無い）
- 「完了」は完了ステータスを持つプロジェクトでだけ出す（完了 = そのステータスへの変更）

Phase 2: Labels、Sprint、Milestone、Custom Fields、Time Tracking、添付。

軽量な変更は Optimistic Update（失敗時はロールバックして Toast）。

---

## 16. Reviews

Project 配下に Reviews を出す。既存の Review API・状態遷移・権限をそのまま使い、
Desktop 独自の Review モデルを作らない。

### 16.1 PR 一覧

`GET …/reviews/pull-requests` の値をそのまま出す: PR 番号、タイトル、作者、ラウンド数、
未解決数、Blocking 数、最終レビュー日時。**可否は出さない**（この API は材料を持たない）。

### 16.2 Review Detail

```text
┌──────────────┬──────────────────────┬────────────────────┐
│ Pull Request │ Findings             │ Finding Detail     │
│ #742 ● 3     │ HIGH                 │ Authentication     │
│ #738 ✓       │ ● Authentication     │ src/auth.rs:82     │
│ #721 ● 1     │ MEDIUM               │ Description        │
│              │ ● Error handling     │ History            │
│              │ LOW                  │ [Actions]          │
│              │ ● Naming             │                    │
└──────────────┴──────────────────────┴────────────────────┘
```

PR → Finding → Detail をキーボードで素早く移動できることを重視する。

### 16.3 Round

同一 PR のラウンドを R1, R2, … で表示: Reviewer、Head SHA、Summary、Findings、Created At。
`reviewer_left_tenant` の表示も出す。ラウンドは変更・削除しない。

### 16.4 Finding

Severity: High / Medium / Low / Nit。State: Open / Fixed / Verified / Deferred / Rejected。
Detail に遷移履歴（actor / 日時 / 状態 / note）を出す。

### 16.5 Finding Actions

**`available_actions`（遷移先 state の一覧）をそのまま Action にする。** Desktop に
遷移規則の表を持たない。

| 遷移先 | ボタン名 |
|---|---|
| `fixed` | Mark as Fixed |
| `verified` | Verify |
| `open` | Reopen（fixed / deferred / rejected のどこからでも同じ） |
| `deferred` | Defer |
| `rejected` | Reject |

`available_actions` が空なら Action を出さない（Verified など）。Deferred で
`deferred_task_id` があれば Task へのリンクを出す。押した結果が 403 / 409 なら
Backend の `message` を Toast に出して再取得する。

### 16.6 Review Gate

`GET …/reviews/summary` の `gate` をそのまま出す。

| `gate` | 表示 |
|---|---|
| `unlinked` | Repository not linked（ゲートとして扱わない旨を出す） |
| `unreviewed` | Not reviewed |
| `blocked` | Blocked · N findings |
| `stale_unknown` | Freshness unknown |
| `outdated` | Review is outdated · reviewed `91ac42f` / current `ab74d21` |
| `ready` | Ready · checked at {pr_head_checked_at} |

判断材料（最新ラウンドの SHA、対象リポジトリ、代行棄却数、確認時刻）も並べる。

### 16.7 Review 作成

Web と同じ規則。指摘はクライアント側 Draft に貯め、Submit で Round + Findings を一括確定する。

- Head SHA は **40 桁小文字 16 進**を送信前に検証する
- 指摘ゼロでの確定も可
- 複数の連携先がある場合は repo / host を指定する
- 確定済みラウンドへ指摘を追加しない。追加は新しいラウンド

---

## 17. Markdown

Summary / Finding body / Task description / Comment は gpui-kit の Markdown で表示する。

---

## 18. Command Palette（Ctrl/Cmd + K）

```text
Create Task        Search Tasks      Open Project
Open Notifications Mark All Read
Open Reviews       Open Current Review   Start Review
Mark Finding as Fixed / Verify / Reopen / Defer / Reject   ← available_actions に応じて出す
Toggle Sidebar     Open Settings     Switch Tenant
```

Context に応じて出す Action を切り替える。

---

## 19. Quick Search（Ctrl/Cmd + P）

MVP の対象は **Projects と Tasks**（選択中テナント内、既存の一覧 API の検索を使う）。
Reviews / PR の横断検索は Backend にテナント横断のエンドポイントが無いので Phase 2。

---

## 20. Quick Add（Phase 2）

グローバルショートカット（既定 Ctrl/Cmd + Shift + Space、変更可）から Main Window を
開かずに Task を作る。既定の作成先は個人プロジェクト（`/v1/users/me/personal-project`）。

---

## 21. Theme

Light / Dark / System。Koyori 側で Semantic Token（background / surface / text /
text_muted / border / accent / success / warning / danger）を定義し、Feature に色を直書きしない。

---

## 22. Settings

| 区分 | 項目 | 保存先 |
|---|---|---|
| General | Launch at Login / Keep Running in Background | ローカル |
| Notifications | Enable Desktop Notifications / Task / Review / Due Date | ローカル（OS 通知の ON/OFF のみ） |
| Notifications | プロジェクトごとの通知設定へのリンク | Web（Backend の `notification_settings`） |
| Appearance | Light / Dark / System | ローカル |
| Keyboard | Keybindings | ローカル |
| Account | Account Information / Devices / Logout | Backend |

---

## 23. エラー処理

| 状況 | 表現 |
|---|---|
| 軽微な通信エラー | Toast |
| 接続障害 | Header の Connection Status + 自動再試行（Blocking Dialog にしない） |
| 401 | 未ログインへ戻し、再承認を促す |
| 403 / 409 | Backend の `message` を Toast |
| 404（通知の対象） | 「no longer exists」を Detail に出す |
| 操作継続不能 | Error View / Dialog |

---

## 24. ローカルキャッシュ（Phase 2）

SQLite。目的は高速起動・オフライン閲覧。オフライン書き込みと競合解決は別仕様。

---

## 25. Auto Update

| OS | 方式 |
|---|---|
| Windows | MSIX の自動更新（配布フィード） |
| macOS | 署名 + notarization 済み DMG。更新は Phase 2（Sparkle 相当を検討） |
| Linux | AppImage。更新は Phase 2 |

3 OS ともコード署名を配布の前提にする。

---

## 26. 決定事項ログ

| 決定 | 理由 |
|---|---|
| 3 OS を MVP に含める | GPUI と gpui-kit が Windows を対応済み。Windows は crates.io のスナップショット版で git 依存なしに使える |
| `gpui` を直接依存しない | gpui-kit が完全固定しているため、2 系統が混ざる |
| Windows の toast クリックは常駐中のみ | 未起動時の受け取りは COM Activator 登録が要り MSIX 前提でも重い。常駐前提なので不要 |
| 配布は MSIX | toast に必要な AUMID と自動更新が付く |
| loopback 認証 | Custom URI Scheme は Linux で登録、Windows でレジストリが要り、他アプリに奪われ得る |
| MVP はポーリング | Backend に Realtime 基盤が無い。`after` カーソルで取り切れるので設計は変わらない |
| 遷移規則と可否判定を持たない | Backend の `available_actions` / `gate` を使う。Web との二重持ちを三重にしない |
| Inbox を置かない | Koyori に対応する概念が無い |
| コメント表示を MVP に入れる | Mention 通知の飛び先で本文が読めないと通知の意味が無い |
| OS 通知の ON/OFF はローカル設定 | Backend の設定は「保存するか」で意味が違う。混ぜない |
| Finding の title を OS 通知に出さない | 機密になり得る。Backend も文言を保存しない |
