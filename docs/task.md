# Koyori Task — Desktop Client 対応仕様書

## 1. 概要

### 1.1 目的

Koyori Desktop から Task・Review のイベントを確実に受信できるよう、Koyori Task 側の
**既存の通知基盤を拡張**し、Desktop 向けの認証と Review API の補強を行う。

Desktop 固有の UI・OS 連携は本仕様の対象外（desktop.txt）。

### 1.2 前提（現状の実装）

本仕様は新設ではなく拡張である。既にあるもの:

| 対象 | 実装 |
|---|---|
| `notifications` / `notification_settings` / `task_watchers` | `docs/features/tasks/5.notifications.md`、`service::notifications` |
| 通知 API | `GET /v1/users/me/notifications`（`unread_count` 込み・offset ページング）、`PATCH …/{id}/read`、`PATCH …/read-all`、`GET/PUT …/notification-settings/{project_id}` |
| 通知の生成 | handler → `service::notifications::notify_*` を同一トランザクションで呼ぶ。`in_app_events` に無い種別は**保存しない** |
| 一覧の認可 | アクセス可能なプロジェクトのタスクに絞った条件を DB クエリに入れる（権限喪失後は行ごと消える） |
| 種別 | `assigned` / `mentioned` / `status_changed` / `comment_added`（`deadline_soon` / `pr_merged` は予約のみ） |
| Bearer 認証 | PAT（1 テナント束縛、`/v1/users/me/*` はセッション専用） |
| Review | ラウンド・指摘・遷移履歴・集計・PR 一覧の API（`docs/features/review-findings.md`） |
| Webhook | GitHub App の `push` / `issues` のみ処理。`pull_request` は捨てている |
| Realtime | **無い**（WebSocket / SSE / pub/sub のいずれも） |

---

## 2. 対象範囲

| 項目 | フェーズ |
|---|---|
| `notifications` の拡張（`project_id` / `target` / `dedupe_key`） | MVP |
| 通知一覧のカーソル化と `after` による catch-up | MVP |
| Review イベントからの通知生成 | MVP |
| Desktop 認証（Device Token + Authorization Code + PKCE） | MVP |
| Finding の `available_actions`、集計の `gate` | MVP |
| Retention（90 日） | MVP |
| Realtime（SSE） | Phase 2 |
| `review.outdated`（`pull_request` webhook） | Phase 2 |
| `deadline_soon`（Scheduler） | Phase 2 |
| PR 作者への通知（GitHub login と利用者の同定） | MVP（§7.1） |

既存の Task / Review API はそのまま使う。Desktop 専用 API は作らない。

---

## 3. Notification モデル（拡張）

既存テーブルへ列を足す。

```text
notifications
  id                UUID PK
  user_id           UUID      受信者（既存）
  task_id           UUID?     既存。task 系の種別だけが持つ
  project_id        UUID      追加。NOT NULL。認可の絞り込みに使う
  notification_type VARCHAR   既存
  payload           JSONB     既存。表示用の非機密データ（actor 名・PR 番号・ラウンド番号など）
  target            JSONB     追加。遷移先（§4）
  dedupe_key        VARCHAR?  追加。冪等化キー（§13）。UNIQUE (user_id, dedupe_key)
  read_at           TIMESTAMPTZ?
  created_at        TIMESTAMPTZ
```

- `project_id` は既存行を `tasks.project_id` から埋めてから NOT NULL にする。
  プロジェクトに属さない通知（テナント招待・端末・セキュリティ等）は現時点で種別が
  無いので nullable にしない。必要になったら `DROP NOT NULL` と §11 の絞り込みに
  `OR project_id IS NULL`（NULL = アカウント単位。受信者に常に見える）を足し、`kind=account`
  を増やす。データ移行は要らない
- **title / body は保存しない**。表示文言は `notification_type` と `payload`、および
  target の解決結果（タスクの `seq_id` / `title`、PR 番号など）からクライアントが組み立てる。
  Finding の title のような機密になり得る文字列は通知行に持たず、詳細は対象 API で
  現在の認可のもとで取る（§11）
- 通知の FK は既存の `task_id`（CASCADE）だけ。Review 系は `target` の ID 参照のみで
  FK を張らない。対象が消えても通知行は残り、対象 API が 404 を返す（§12）
- `(user_id, created_at DESC, id)` の索引を張る（カーソルの並び順）

---

## 4. Target

```json
{ "type": "task", "task_id": "…" }
{ "type": "review", "review_id": "…" }
{ "type": "review_finding", "review_id": "…", "finding_id": "…" }
{ "type": "review_finding", "review_id": "…", "finding_id": "…", "deferred_task_id": "…" }
```

- `tenant_id` / `project_id` は通知行の列（`project_id`）とレスポンスの `project` 要約
  （`tenant_id` / `project_id` / `key`）で返す。target の中に重複して持たない
- PR は `review_id` から辿る（`reviews` に `pr_number` / `repo_owner` / `repo_name` がある）。
  `pull_request_id` はまだ実装に無いので使わない
- Desktop 固有の `koyori://…` は保存しない。URI への変換はクライアントの責務

---

## 5. Notification Type

既存名はそのまま維持する（`notification_settings.in_app_events` に文字列で保存済みのため
改名しない）。Review 系だけ `review.` 接頭辞で足す。

| type | 状態 | 受信者 | 発火 |
|---|---|---|---|
| `assigned` | 既存 | 担当者 | 担当追加 |
| `mentioned` | 既存 | メンションされた人 | コメント投稿 |
| `status_changed` | 既存 | ウォッチャー | ステータス変更 |
| `comment_added` | 既存 | ウォッチャー | コメント投稿 |
| `deadline_soon` | Phase 2 | 担当者 + ウォッチャー | Scheduler（§14） |
| `review.round_created` | 追加 | §7.1 | ラウンド一括作成 |
| `review.finding_fixed` | 追加 | 指摘のラウンド作成者 | open → fixed |
| `review.finding_reopened` | 追加 | `fixed_by` | fixed → open |
| `review.finding_verified` | 追加 | `fixed_by` | fixed → verified |
| `review.finding_deferred` | 追加 | 指摘のラウンド作成者 | open → deferred |
| `review.outdated` | Phase 2 | 最新ラウンドの作成者 | PR HEAD 変更（§8） |

作らないもの:

- `review.finding_created` / `review.blocking_finding_created`: ラウンドは指摘込みで一括作成
  されるので `review.round_created` 1 本に件数（`findings` / `blocking`）を載せる。個別に
  出すと同一操作で複数の OS 通知になる
- `review.finding_rejected` / `rejected → open`: 遷移できるのが指摘を出した本人だけ
  （代行を除く）で、actor = 通知先になる
- `task.mentioned` 等の改名: 移行に見合う価値が無い

`KNOWN_EVENT_TYPES` と `DEFAULT_IN_APP_EVENTS` に Review 系を追加する。

---

## 6. 生成の経路

既存どおり handler → `service::notifications` を**同一トランザクション**で呼ぶ。
Domain Event 層や Outbox は導入しない。

```text
Finding open → fixed
  + notifications INSERT      ← 同一 TX
COMMIT
  → （Phase 2）Realtime publish（best-effort。失敗は catch-up で吸収）
```

- 通知行と状態変更が同時に commit されるので、「fixed になったのに通知だけ無い」は起きない
- Realtime の送信失敗は §10 の catch-up が救うので、Outbox で守る対象が無い
- apalis のジョブ本文は Postgres に平文で残るため、通知の文言や finding の内容をジョブに
  載せない（既存の地雷）。ジョブが必要になる Phase 2 の種別（§8, §14）は ID だけを載せる

`actor == recipient` は生成しない（既存 `notify_*` と同じ）。Actor が無いシステム起点
（Scheduler / webhook）はこの規則の対象外。

---

## 7. Review 通知の宛先

### 7.1 `review.round_created`

宛先は 2 経路で決め、同じ `dedupe_key = review.round_created:{review_id}` で冪等に入れる
（`UNIQUE (user_id, dedupe_key)`。同じ人が両方に当たっても 1 件）。

| 経路 | 宛先 | タイミング |
|---|---|---|
| handler（同一 TX） | 同じ PR の**過去ラウンドの参加者**（各ラウンドの `reviewer_id`、指摘の `fixed_by`、遷移の `actor_id`）から actor を除いたもの | ラウンド作成時 |
| 要約ジョブ | **PR 作者**。`cache_pr_meta` が GitHub から取った `user.login` を `oauth_connections`（`provider = 'github'`、`provider_login` を大文字小文字無視で一致）で利用者に引き、プロジェクトに入れる人だけ | ジョブ実行時。対象はその PR の**最新ラウンド**だけ |

- payload: `pr_number` / `round` / `findings` / `blocking` / `reviewer`（username）
- R1 は過去ラウンドが無いので handler 経路の宛先は空。主用途（AI が CLI で R1 を出し、
  開発者が通知で気づく）はジョブ経路の PR 作者通知で成立する
- ジョブは (project, repo, pr) 単位で合流し「どのラウンドの作成で走ったか」を知らない
  ので、最新ラウンドだけを対象にする。連続作成で 1 本飛んでも次のラウンドで届く
- 作者 = ラウンド作成者（自己レビュー）は actor 除外で生成しない
- 連携なしプロジェクト、作者が GitHub 連携していない利用者、GitHub 取得失敗のときは
  作者通知は出ない（要約コメントと同じ best-effort。`enqueue_best_effort` の失敗も同様）
- 通知にジョブが載せるのは ID だけ（既存の `ReviewSummaryJob` に追加は不要。
  ラウンドは DB から引く）

### 7.2 遷移

`update_review_finding_state` の遷移ごとに §5 の表で 1 件生成する。宛先は
`reviews.reviewer_id` / `review_findings.fixed_by` を読むだけで決まる。
`review.finding_deferred` の target には `deferred_task_id` を含める（起票直後の値を使う）。

宛先がプロジェクトに入れなくなっている場合は生成しない（既存 `notify_watchers` と同じ
`project_accessible_user_ids` の判定を通す）。

---

## 8. `review.outdated`（Phase 2）

既存の webhook は `push` と `issues` しか処理していないので、**`pull_request` イベントの
購読と処理を新設**する（GitHub App の subscribed events も変更）。

- `action = synchronize` を受けたら、その PR の最新ラウンドの `head_sha` と
  `pull_request.head.sha` を比較し、不一致なら生成する
- あわせて `reviews.pr_head_sha` / `pr_head_checked_at` を更新する（現状は要約ジョブが
  ラウンド作成・遷移時に取るキャッシュで、push では更新されない。これが更新される
  ようになれば集計の「鮮度不明」も減る）
- `dedupe_key = review.outdated:{project_id}:{repo_owner}/{repo_name}:{pr_number}:{head_sha}`。
  同じ HEAD で再配信されても 2 件目を作らない
- `push` イベントから PR を引くことはできない（PR の head branch を控えていない）ので、
  `push` で代用しない

---

## 9. Notification API（変更）

パスは既存の `/v1/users/me` 配下。アカウント全体（テナント横断）で返す（既存どおり）。

| メソッド | パス | 変更 |
|---|---|---|
| `GET` | `/v1/users/me/notifications` | ページングを**カーソル化**。`unread_count` は維持 |
| `PATCH` | `/v1/users/me/notifications/{id}/read` | 既存のまま |
| `PATCH` | `/v1/users/me/notifications/read-all` | 既存のまま |
| `GET` / `PUT` | `/v1/users/me/notification-settings/{project_id}` | 既存のまま。Review 系の種別を受け付ける |

### 9.1 一覧

```text
GET /v1/users/me/notifications?unread=true&kind=review&limit=50&cursor=…
GET /v1/users/me/notifications?after=…
```

| クエリ | 意味 |
|---|---|
| `unread` | 未読のみ |
| `kind` | `task` / `review`。種別の分類（`review.` 接頭辞の有無） |
| `limit` | 既定 50、上限 100 |
| `cursor` | このカーソルより**古い**行（通常のページ送り） |
| `after` | このカーソルより**新しい**行（catch-up。§10）。`cursor` と同時指定は 400 |

- **並びは `created_at DESC, id DESC` のみ**。既存の「未読優先」は廃止する。既読化で
  行の順位が変わるとカーソルが成立しない。未読を上に出したい画面は `unread=true` で
  別に引く（破壊的変更。Web の利用箇所は未実装なので影響なし）
- カーソルは `common::cursor`（`(created_at, id)` を base64）。壊れた値は 400
- `id` は UUID v4 で順序を持たないので、単独ではカーソルにしない

レスポンス:

```json
{
  "unread_count": 3,
  "next_cursor": "…",
  "notifications": [
    {
      "id": "…",
      "notification_type": "review.finding_fixed",
      "project": { "tenant_id": "…", "id": "…", "key": "TASK" },
      "task": null,
      "payload": { "actor": "yupix", "pr_number": 742, "round": 2 },
      "target": { "type": "review_finding", "review_id": "…", "finding_id": "…" },
      "read_at": null,
      "created_at": "…"
    }
  ]
}
```

既読解除・単独の unread-count エンドポイントは作らない（`limit=1` の一覧で足りる）。

---

## 10. Catch-up

クライアントは最後に受信した通知のカーソル（最新行の `(created_at, id)`）を保持し、
起動時・再接続時に `after=` で差分を取る。

```text
最後に受信 N100 → オフライン → N101..N103 → 起動
GET ?after=cursor(N100) → N101, N102, N103
```

`unread=true` では「既読だが未受信」を取り漏らすので、catch-up は必ず `after` を使う。
`after` の結果が `limit` を超える場合は `next_cursor` で続きを引く。

MVP の Desktop はこの `after` を **30 秒間隔のポーリング**で叩く。Realtime は Phase 2。

---

## 11. 認可

- 通知は受信者本人のみ。他人の ID を指定しても 404
- 一覧は既存どおり**アクセス可能なプロジェクト**（`project_id`）に絞る条件を DB クエリに入れる。
  後から権限を失ったプロジェクトの通知は行ごと出ない。`unread_count` も同じ条件
- 通知行に機密文言を持たない（§3）ので、絞り込みが漏れても出るのは種別と ID のみ
- 詳細は対象 API（Task / Review / Finding）が現在の認可で判定する

---

## 12. 対象の削除

対象が消えても通知は消さない（Review 系は FK 無し、task は soft delete）。対象 API は
通常の 404 を返し、クライアントが「This task no longer exists.」等を表示する。

---

## 13. 冪等性

`dedupe_key` を持つ種別は `INSERT … ON CONFLICT (user_id, dedupe_key) DO NOTHING`。
対象は webhook / ジョブ / Scheduler 起点の種別（`review.round_created` のジョブ経路、
`review.outdated`、`deadline_soon`）。`review.round_created` は handler 経路にも同じキーを
付ける（§7.1 の二重防止）。それ以外の handler 起点の種別は同一 TX で 1 回しか走らないので付けない。

---

## 14. `deadline_soon`（Phase 2）

apalis の cron ジョブで 1 時間ごとに走らせ、`due_date` が 24 時間以内のタスクの担当者と
ウォッチャーへ生成する。`dedupe_key = deadline_soon:{task_id}:{due_date}`。
種別名は既存の予約名を使う。

---

## 15. Retention

90 日より古い通知を日次の cron ジョブで削除する。期間は設定値（`NOTIFICATION_RETENTION_DAYS`）。
通知の削除は監査ログ・遷移履歴に影響しない。

---

## 16. Realtime（Phase 2）

```text
GET /v1/users/me/notifications/stream        text/event-stream
```

- **SSE** を使う。通知は一方向で足り、`Last-Event-ID` がそのまま §10 のカーソルになる
  （接続時に `Last-Event-ID` を受けたら `after` と同じ差分を先に流してから live に入る）。
  WebSocket は双方向が要らないので採用しない
- イベントは `notification`（id / type / project / target。文言は持たない）のみ。
  `notification.read` は作らない。同期相手（Web の通知 UI）がまだ無い
- 認証は Bearer（Device Token）。セッション Cookie でも通す場合は既存の Origin 検査を
  掛ける（CSRF ミドルウェアは Bearer 付きだけを除外している）
- 複数インスタンス構成では Redis pub/sub で中継する（chan: `notifications:{user_id}`）。
  publish は commit 後の best-effort。取りこぼしは `Last-Event-ID` で埋まる
- 切断は正常系。クライアントは指数バックオフで再接続し、再接続時は必ず §10 を通す

---

## 17. Desktop 認証

### 17.1 方式

PAT は使えない。1 テナント束縛で、`/v1/users/me/*` がセッション専用のため
テナント横断の通知一覧を取れない。`personal-access-tokens-authz.md` の
「テナント非紐づけ PAT は採用しない」を覆さず、**Device Token を別種別として足す**。

```text
device_tokens
  id            UUID PK
  user_id       UUID
  name          VARCHAR   端末名（クライアントが申告。表示用）
  token_hash    VARCHAR   SHA-256（PAT と同じ）
  token_last_four VARCHAR
  expires_at    TIMESTAMPTZ   発行から 90 日
  last_used_at  TIMESTAMPTZ?
  revoked_at    TIMESTAMPTZ?
  created_at    TIMESTAMPTZ
```

- `AuthMethod::DeviceToken { token_id }` を追加。**スコープ・テナント束縛はセッションと同等**
  （`require_scope` は常に通過、`has_tenant_access` で所属判定）。`require_session` の
  口（PAT 管理・テナント作成など）は通さない。通知 API・`/v1/users/me/*` は
  セッションと Device Token の両方を通すように `require_session` を緩める
- `users.sessions_revoked_at` を Device Token にも適用する（パスワード変更で全端末が落ちる）
- Bearer の接頭辞で PAT と区別する（PAT は既存の接頭辞、Device Token は `kdt_`）

### 17.2 フロー（Authorization Code + PKCE、loopback）

```text
Desktop                                   Browser / Web              Backend
  │ 127.0.0.1:{port} で待ち受け
  │ code_verifier / code_challenge / state を生成
  ├─ 開く ─→ {web}/desktop/authorize?port=&code_challenge=&state=&name=
  │                                          │ 未ログインなら通常ログイン（2FA 含む）
  │                                          │ 「Koyori Desktop を承認」画面
  │                                          ├─ POST /v1/desktop/auth/codes ──→ セッション必須
  │                                          │      {code_challenge, name}     half-authed は 403
  │                                          │←──── {code}
  │←─ redirect http://127.0.0.1:{port}/callback?code=&state= ─┤
  │ state を検証
  ├─ POST /v1/desktop/auth/token {code, code_verifier} ────────────────────→
  │←──────────────────────────── {token, expires_at} ──────────────────────┤
  │ OS credential store へ保存
```

| 項目 | 規則 |
|---|---|
| code | 32 バイト乱数。Redis に TTL 5 分で保存（`user_id` / `code_challenge` / `name`）。**GETDEL で一度きり** |
| PKCE | `S256`。`code_verifier` の検証に失敗したら code は消費済みとして扱う |
| redirect | **loopback のみ**（`http://127.0.0.1:{port}/callback`）。Custom URI Scheme は使わない。Linux で登録が要り、Windows でレジストリ書き込みが要り、他アプリに奪われ得る |
| 発行の口 | `POST /v1/desktop/auth/codes` はセッション専用。Bearer は 403 |
| 交換の口 | `POST /v1/desktop/auth/token` は未認証で叩く（code が資格）。レート制限を掛ける |
| 有効期限 | 90 日。期限切れは 401 で、Desktop はブラウザ承認をやり直す |

長寿命の資格情報を URL に載せない（URL に出るのは短寿命の code だけ）。

### 17.3 端末管理

| メソッド | パス | 認証 |
|---|---|---|
| `GET` | `/v1/users/me/devices` | セッション / Device Token |
| `DELETE` | `/v1/users/me/devices/{id}` | セッション / Device Token（自分自身も可 = ログアウト） |

Web のアカウント設定に端末一覧と失効を出す。失効は `revoked_at` を立てる（行は残す）。

---

## 18. Review API の補強

Desktop 専用の Review API は作らない。既存に 2 つ足す。

### 18.1 `available_actions`

Finding のレスポンス（`GET …/reviews/{id}` の指摘、`GET …/review-findings`）に、
要求者がいま遷移できる**状態の一覧**を返す。

```json
{ "id": "…", "state": "fixed", "available_actions": ["verified", "open"] }
```

- 値は遷移先の `state`（動詞ではない）。API の遷移は `PATCH … {state}` なので 1:1 になる
- 判定は `service::reviews::ensure_transition_allowed` を候補ごとに評価するだけ。
  規則の表を増やさない
- **これが無いと遷移規則が 3 重持ちになる**。既に backend と Web（`lib/review-findings.ts`）で
  二重持ちで、仕様書 §8 が「片方だけ変えると壊れる」と警告している。Desktop は
  `available_actions` だけを見る。Web も追って同じ値に切り替える（別 PR）

### 18.2 `gate`

`GET …/reviews/summary` に判定を列挙で返す。

| `gate` | 条件（上から順に最初に当たったもの） |
|---|---|
| `unlinked` | 集計対象のリポジトリが確定しない（連携なし） |
| `unreviewed` | ラウンドが 0 件 |
| `blocked` | open / fixed の High・Medium がある |
| `stale_unknown` | `cached_pr_head_sha` が無い |
| `outdated` | `cached_pr_head_sha` ≠ `latest_head_sha` |
| `ready` | 上のいずれでもない（`pr_head_checked_at` を併記する） |

既存の 6 値と同じ規則。既存フィールド（件数・SHA・確認時刻・代行棄却数）は残す。
Desktop / Web はこの値を表示に使い、判定を再計算しない。権威のゲートは引き続き
CLI の `--head` 照合と branch protection。

---

## 19. OpenAPI

追加・変更する型をすべて既存の `openapi.json` へ出す。Desktop は OpenAPI から型を生成する。

- `NotificationItem`（`project` / `target` 追加、`next_cursor`）
- `NotificationTarget`（type ごとの oneOf）
- `DeviceToken` / `DesktopAuthCodeRequest` / `DesktopAuthTokenRequest`
- `ReviewFindingResponse.available_actions`、`ReviewSummaryResponse.gate`

---

## 20. Web 版

通知基盤は Desktop 専用に閉じない。Web のヘッダー通知 UI（5.notifications.md の Phase B、
未実装）は同じ一覧 API と `kind` フィルタを使う。Web 側の実装は本仕様の範囲外。

---

## 21. セキュリティ

- 通知 API・端末管理 API は認証必須。他人の通知・端末は 404
- 通知行に機密文言を持たない。対象の閲覧は対象 API の現在の認可で判定
- Authorization Code は短寿命・一度きり・PKCE 必須
- Device Token は hash 保存、期限 90 日、端末ごとに失効可、`sessions_revoked_at` の対象
- 交換エンドポイントにレート制限
- Bearer / code をログに出さない
- SSE（Phase 2）は Bearer 認証。Cookie を通すなら Origin 検査

---

## 22. 追従する仕様書

実装時に同じ PR で更新する。

- `docs/features/tasks/5.notifications.md`: 列追加、カーソル化、並び順の変更、Review 系の
  種別、Retention。**既存の API 表（POST / PUT）が実装（PATCH）とズレているので直す**
- `docs/features/review-findings.md`: `available_actions` / `gate`、通知の発火点、
  `pull_request` webhook（Phase 2）
- `apps/backend/docs/personal-access-tokens-authz.md`: Device Token を認証方式の表に足す
- `apps/backend/docs/README.md`: 索引

---

## 23. 決定事項ログ

| 決定 | 理由 |
|---|---|
| 既存テーブルを拡張し、種別名を改名しない | `in_app_events` に文字列で保存済み。改名の移行に見合う価値が無い |
| title / body を保存しない | Finding の title は機密になり得る。既存の `task` 要約と同じく表示時に解決すれば、権限喪失後の漏れも既存の絞り込みで防げる |
| Preference の意味を変えない（無効な種別は保存しない） | 既存挙動。「保存するが表示しない」に変えると設定の意味が二重になる。OS 通知の ON/OFF は Desktop のローカル設定で持つ |
| Preference はプロジェクト単位のまま | ユーザー単位の新テーブルを足すと 2 段の AND になり、どちらで止まったか分からなくなる |
| 並びを新着順のみにする | 未読優先はカーソルと両立しない |
| Outbox を入れない | 通知は同一 TX、Realtime の失敗は catch-up で吸収。守る対象が無い |
| Realtime は SSE、MVP はポーリング | 一方向で足り、`Last-Event-ID` が catch-up と一致する。基盤が無いので MVP から外しても設計は変わらない |
| `finding_created` / `blocking_finding_created` を作らない | 一括作成なので `round_created` に件数を載せれば足りる |
| PR 作者への通知を MVP に入れ、要約ジョブで生成する | R1 が 0 人だと主用途（AI の R1 を開発者が通知で知る）が成立しない。`oauth_connections.provider_login` で同定でき、ジョブは既に PR 作者を取得している。handler では作成時点で作者が分からない |
| `project_id` を NOT NULL のままにする | プロジェクト外の通知種別がまだ無い。緩和は制約変更と絞り込み 1 句で済み、データ移行が要らない |
| `review.outdated` は Phase 2 | `pull_request` webhook が無く、`push` から PR を引けない |
| Device Token を PAT と別種別にする | PAT の「テナント非紐づけは採用しない」を覆さない。セッション相当の権限を Bearer で持たせる |
| loopback + PKCE | Custom URI Scheme は Linux で登録が要り、Windows でレジストリ書き込みが要り、他アプリに奪われ得る |
| `available_actions` / `gate` を MVP に入れる | 3 つ目のコピーを作らせない。仕様書が既に二重持ちの危険を書いている |
