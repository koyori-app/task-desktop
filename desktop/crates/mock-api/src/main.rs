//! Spec-conformant mock of the Koyori backend (`docs/task.md`) for manual
//! desktop testing while the Device Token flow is not deployed.
//!
//! Run: `cargo run -p mock-api` then start the app with
//! `KOYORI_API_BASE=http://127.0.0.1:4199/api` + `KOYORI_DEV_TOKEN=dev`.

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, patch, post},
};
use serde_json::{Value, json};
use uuid::Uuid;

const TENANT: &str = "11111111-1111-1111-1111-111111111111";
const TENANT2: &str = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
const PROJECT: &str = "22222222-2222-2222-2222-222222222222";
const PROJECT2: &str = "dddddddd-dddd-dddd-dddd-dddddddddddd";
const PERSONAL: &str = "33333333-3333-3333-3333-333333333333";
const USER_YUPIX: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const USER_ALICE: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
const ST_BACKLOG: &str = "44444444-4444-4444-4444-444444444444";
const ST_TODO: &str = "55555555-5555-5555-5555-555555555555";
const ST_DOING: &str = "66666666-6666-6666-6666-666666666666";
const ST_DONE: &str = "77777777-7777-7777-7777-777777777777";
const REVIEW_1: &str = "88888888-8888-8888-8888-888888888888";
const REVIEW_2: &str = "99999999-9999-9999-9999-999999999999";

fn user(id: &str, name: &str) -> Value {
    json!({"id": id, "username": name, "avatar_url": null})
}

fn status(id: &str, name: &str, color: &str, pos: i32, done: bool, default_done: bool) -> Value {
    json!({
        "id": id, "project_id": PROJECT, "name": name, "color": color,
        "position": pos, "is_default": name == "Backlog",
        "is_done_state": done, "is_default_done": default_done,
        "created_at": "2026-09-01T00:00:00Z",
    })
}

fn task(
    id: Uuid,
    seq: i32,
    title: &str,
    st: &str,
    priority: &str,
    assignees: Value,
    project: &str,
) -> Value {
    json!({
        "id": id, "seq_id": seq, "title": title,
        "description": format!("{title} の説明文。\n\n- 箇条書き\n- **太字** も含む"),
        "status_id": st, "priority": priority, "progress_pct": 0,
        "project_id": project, "assignees": assignees,
        "labels": [], "custom_field_values": [],
        "milestone_id": null, "sprint_id": null, "parent_task_id": null,
        "estimated_minutes": null, "soft_deadline": null, "hard_deadline": null,
        "is_archived": false, "completed_at": null, "deleted_at": null,
        "created_by": user(USER_YUPIX, "yupix"),
        "created_at": "2026-09-20T10:00:00Z", "updated_at": "2026-09-22T10:00:00Z",
    })
}

#[derive(Default)]
struct Mock {
    tasks: Vec<Value>,
    comments: Vec<Value>,
    notifications: Vec<Value>,
    findings: Vec<Value>,
    devices: Vec<Value>,
    reviews: Vec<Value>,
    next_seq: i32,
}

impl Mock {
    fn seed() -> Self {
        let t1 = Uuid::new_v4();
        let t2 = Uuid::new_v4();
        let t3 = Uuid::new_v4();
        let t4 = Uuid::new_v4();
        let me = json!([{"role": "assignee", "user": user(USER_YUPIX, "yupix")}]);
        let both = json!([
            {"role": "assignee", "user": user(USER_YUPIX, "yupix")},
            {"role": "reviewer", "user": user(USER_ALICE, "alice")},
        ]);
        let mut tasks = vec![
            task(t1, 1, "モック環境の動作確認", ST_TODO, "High", me.clone(), PROJECT),
            task(t2, 2, "通知センターの表示確認", ST_DOING, "Medium", both, PROJECT),
            task(t3, 3, "コメント投稿のテスト", ST_BACKLOG, "Low", json!([]), PROJECT),
            task(t4, 4, "完了済みタスクの例", ST_DONE, "Trivial", me.clone(), PROJECT),
            task(Uuid::new_v4(), 1, "別テナントのタスク", ST_TODO, "Medium", me, PROJECT2),
            task(Uuid::new_v4(), 2, "OTHER プロジェクトの件", ST_DOING, "High", json!([]), PROJECT2),
        ];
        tasks[3]["completed_at"] = json!("2026-09-22T15:00:00Z");

        let comments = vec![
            json!({
                "id": Uuid::new_v4(), "body": "モックのコメント本文です。",
                "created_at": "2026-09-21T09:00:00Z", "updated_at": "2026-09-21T09:00:00Z",
                "is_deleted": false, "user": {"id": USER_ALICE, "name": "alice", "avatar_url": null},
                "replies": [],
            }),
            json!({
                "id": Uuid::new_v4(), "body": "返信付きスレッド。",
                "created_at": "2026-09-21T10:00:00Z", "updated_at": "2026-09-21T10:00:00Z",
                "is_deleted": false, "user": {"id": USER_YUPIX, "name": "yupix", "avatar_url": null},
                "replies": [{
                    "id": Uuid::new_v4(), "body": "スレッド内返信。",
                    "created_at": "2026-09-21T10:30:00Z", "updated_at": "2026-09-21T10:30:00Z",
                    "is_deleted": false,
                    "user": {"id": USER_ALICE, "name": "alice", "avatar_url": null},
                }],
            }),
        ];

        let n_task = |id: &Uuid, ty: &str, payload: Value, target: Value, read: bool| {
            json!({
                "id": Uuid::new_v4(), "notification_type": ty,
                "project": {"tenant_id": TENANT, "id": PROJECT, "key": "MOCK"},
                "task": {"id": id, "seq_id": 1, "title": "モック環境の動作確認"},
                "payload": payload, "target": target,
                "read_at": if read { json!("2026-09-22T12:00:00Z") } else { Value::Null },
                "created_at": "2026-09-23T08:00:00Z",
                "cursor": format!("mock-cursor-{}", Uuid::new_v4()),
            })
        };
        let notifications = vec![
            n_task(&t1, "assigned", json!({"actor": "alice"}), json!({"type": "task", "task_id": t1}), false),
            n_task(&t2, "comment", json!({"actor": "alice", "excerpt": "モックのコメント本文です。"}), json!({"type": "task", "task_id": t2}), false),
            json!({
                "id": Uuid::new_v4(), "notification_type": "review.finding_open",
                "project": {"tenant_id": TENANT, "id": PROJECT, "key": "MOCK"},
                "task": null,
                "payload": {"actor": "alice", "pr_number": 42, "severity": "high", "title": "エラーハンドリングが不足"},
                "target": {"type": "review_finding", "review_id": REVIEW_1, "finding_id": "11111111-2222-3333-4444-555555555555"},
                "read_at": null, "created_at": "2026-09-23T09:00:00Z",
                "cursor": format!("mock-cursor-{}", Uuid::new_v4()),
            }),
            n_task(&t3, "mention", json!({"actor": "alice"}), json!({"type": "task", "task_id": t3}), true),
        ];

        let f1 = Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
        let finding = |id: Uuid, sev: &str, st: &str, title: &str, file: &str, line: i32, actions: &[&str]| {
            json!({
                "id": id, "review_id": REVIEW_1, "pr_number": 42, "round": 1,
                "severity": sev, "title": title, "body": format!("{title} の詳細。"),
                "file": file, "line": line, "state": st,
                "fixed_by": null, "deferred_task_id": null,
                "transitions": [], "available_actions": actions,
                "created_at": "2026-09-22T11:00:00Z", "updated_at": "2026-09-22T11:00:00Z",
            })
        };
        let findings = vec![
            finding(f1, "high", "open", "エラーハンドリングが不足", "src/main.rs", 42, &["fixed", "deferred", "rejected"]),
            finding(Uuid::new_v4(), "medium", "open", "命名が曖昧", "src/lib.rs", 10, &["fixed", "deferred", "rejected"]),
            finding(Uuid::new_v4(), "low", "fixed", "不要な clone", "src/util.rs", 88, &[]),
            finding(Uuid::new_v4(), "nit", "open", "コメントの typo", "README.md", 3, &["fixed", "deferred", "rejected"]),
        ];

        let review = |id: &str, round: i32| {
            json!({
                "id": id, "project_id": PROJECT, "pr_number": 42,
                "pr_title": "feat: add mock API", "pr_author": "alice",
                "round": round, "head_sha": "0123456789abcdef0123456789abcdef01234567",
                "summary": format!("R{round} の総評。"),
                "reviewer": user(USER_YUPIX, "yupix"), "reviewer_left_tenant": false,
                "finding_count": 2, "created_at": "2026-09-22T11:00:00Z",
            })
        };

        Self {
            tasks,
            comments,
            notifications,
            findings,
            reviews: vec![review(REVIEW_1, 1), review(REVIEW_2, 2)],
            devices: vec![
                json!({"id": Uuid::new_v4(), "name": "koyori-mock (dev)", "last_used_at": "2026-09-23T00:00:00Z", "expires_at": "2027-09-23T00:00:00Z", "created_at": "2026-09-01T00:00:00Z"}),
                json!({"id": Uuid::new_v4(), "name": "other-device", "last_used_at": null, "expires_at": "2027-01-01T00:00:00Z", "created_at": "2026-08-01T00:00:00Z"}),
            ],
            next_seq: 5,
        }
    }
}

type Shared = Arc<Mutex<Mock>>;

fn err(status: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"message": msg})))
}

fn find_task(m: &Mock, id: Uuid) -> Option<usize> {
    m.tasks
        .iter()
        .position(|t| t["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()) == Some(id))
}

// ---- handlers ----

async fn tenants() -> Json<Value> {
    Json(json!([
        {
            "id": TENANT, "display_id": "mock", "name": "Mock Tenant",
            "description": "ローカルモック", "icon_url": "", "icon_emoji": "🧪",
            "member_role": "Admin", "membership": "Owner",
            "owner_id": USER_YUPIX, "require_2fa": false, "drive_quota_bytes": null,
        },
        {
            "id": TENANT2, "display_id": "other", "name": "Other Tenant",
            "description": "2つ目のテナント", "icon_url": "", "icon_emoji": "🏢",
            "member_role": "Member", "membership": "Member",
            "owner_id": USER_ALICE, "require_2fa": false, "drive_quota_bytes": null,
        },
    ]))
}

async fn projects(Path(tenant): Path<String>) -> Json<Value> {
    let mut v = vec![
        json!({"id": PERSONAL, "tenant_id": tenant, "key": "PERSONAL", "name": "Personal",
            "description": "", "icon_url": null, "icon_emoji": null,
            "is_personal": true, "personal_owner_id": USER_YUPIX}),
    ];
    if tenant == TENANT {
        v.insert(0, json!({"id": PROJECT, "tenant_id": TENANT, "key": "MOCK", "name": "Mock Project",
            "description": "", "icon_url": null, "icon_emoji": "📦",
            "is_personal": false, "personal_owner_id": null}));
    } else {
        v.insert(0, json!({"id": PROJECT2, "tenant_id": TENANT2, "key": "OTH", "name": "Other Project",
            "description": "", "icon_url": null, "icon_emoji": "📁",
            "is_personal": false, "personal_owner_id": null}));
    }
    Json(json!(v))
}

async fn personal_project(Path(tenant): Path<String>) -> Json<Value> {
    Json(json!({"id": PERSONAL, "tenant_id": tenant, "key": "PERSONAL", "name": "Personal",
        "description": "", "icon_url": null, "icon_emoji": null,
        "is_personal": true, "personal_owner_id": USER_YUPIX}))
}

async fn statuses(Path((_, p)): Path<(String, String)>) -> Json<Value> {
    let mk = |id: &str, name: &str, color: &str, pos: i32, done: bool, dd: bool| {
        let mut s = status(id, name, color, pos, done, dd);
        s["project_id"] = json!(p);
        s
    };
    Json(json!([
        mk(ST_BACKLOG, "Backlog", "#8b949e", 0, false, false),
        mk(ST_TODO, "Todo", "#1f6feb", 1, false, false),
        mk(ST_DOING, "In Progress", "#d29922", 2, false, false),
        mk(ST_DONE, "Done", "#238636", 3, true, true),
    ]))
}

async fn assignable_users() -> Json<Value> {
    Json(json!([user(USER_YUPIX, "yupix"), user(USER_ALICE, "alice")]))
}

fn tenant_of_project(p: &str) -> &str {
    if p == PROJECT2 { TENANT2 } else { TENANT }
}

async fn list_tasks(
    State(m): State<Shared>,
    Path((_, p)): Path<(String, String)>,
) -> Json<Value> {
    let m = m.lock().unwrap();
    let tasks: Vec<Value> = m
        .tasks
        .iter()
        .filter(|t| t["project_id"] == p)
        .cloned()
        .collect();
    Json(json!({"tasks": tasks, "total": tasks.len(), "next_cursor": null}))
}

async fn my_tasks(State(m): State<Shared>, Path(tenant): Path<String>) -> Json<Value> {
    let m = m.lock().unwrap();
    let items: Vec<Value> = m
        .tasks
        .iter()
        .filter(|t| {
            tenant_of_project(t["project_id"].as_str().unwrap_or_default()) == tenant
                && t["assignees"].as_array().is_some_and(|a| {
                    a.iter().any(|x| x["user"]["id"] == USER_YUPIX)
                })
        })
        .map(|t| {
            let st_name = match t["status_id"].as_str().unwrap_or_default() {
                ST_BACKLOG => ("Backlog", "#8b949e"),
                ST_TODO => ("Todo", "#1f6feb"),
                ST_DOING => ("In Progress", "#d29922"),
                _ => ("Done", "#238636"),
            };
            let pid = t["project_id"].as_str().unwrap_or_default();
            let (key, name) = if pid == PROJECT2 {
                ("OTH", "Other Project")
            } else if pid == PERSONAL {
                ("PERSONAL", "Personal")
            } else {
                ("MOCK", "Mock Project")
            };
            json!({
                "id": t["id"], "seq_id": t["seq_id"],
                "seq_key": format!("{key}-{}", t["seq_id"].as_i64().unwrap_or(0)),
                "title": t["title"], "priority": t["priority"],
                "status": {"id": t["status_id"], "name": st_name.0, "color": st_name.1},
                "project": {"id": pid, "key": key, "name": name, "is_personal": pid == PERSONAL},
                "is_personal": pid == PERSONAL,
                "soft_deadline": t["soft_deadline"], "hard_deadline": t["hard_deadline"],
            })
        })
        .collect();
    Json(json!({"tasks": items, "total": items.len()}))
}

async fn get_task(State(m): State<Shared>, Path((_, _, id)): Path<(String, String, Uuid)>) -> impl IntoResponse {
    let m = m.lock().unwrap();
    match find_task(&m, id) {
        Some(i) => Json(m.tasks[i].clone()).into_response(),
        None => err(StatusCode::NOT_FOUND, "task not found").into_response(),
    }
}

async fn create_task(
    State(m): State<Shared>,
    Path((_, p)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let mut m = m.lock().unwrap();
    let seq = m.next_seq;
    m.next_seq += 1;
    let assignees = body["assignees"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| {
                    let uid = x["user_id"].as_str().unwrap_or(USER_YUPIX);
                    let name = if uid == USER_YUPIX { "yupix" } else { "alice" };
                    json!({"role": x["role"], "user": user(uid, name)})
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut t = task(
        Uuid::new_v4(),
        seq,
        body["title"].as_str().unwrap_or("(untitled)"),
        body["status_id"].as_str().unwrap_or(ST_TODO),
        body["priority"].as_str().unwrap_or("Medium"),
        json!(assignees),
        &p,
    );
    if !body["description"].is_null() {
        t["description"] = body["description"].clone();
    }
    m.tasks.push(t.clone());
    Json(t).into_response()
}

async fn update_task(
    State(m): State<Shared>,
    Path((_, _, id)): Path<(String, String, Uuid)>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let mut m = m.lock().unwrap();
    let Some(i) = find_task(&m, id) else {
        return err(StatusCode::NOT_FOUND, "task not found").into_response();
    };
    let t = &mut m.tasks[i];
    for key in ["title", "description", "priority", "progress_pct", "status_id",
        "soft_deadline", "hard_deadline", "estimated_minutes", "is_archived",
        "milestone_id", "sprint_id", "parent_task_id"]
    {
        if !body[key].is_null() {
            t[key] = body[key].clone();
        }
    }
    if let Some(a) = body["assignees"].as_array() {
        t["assignees"] = json!(a.iter().map(|x| {
            let uid = x["user_id"].as_str().unwrap_or(USER_YUPIX);
            let name = if uid == USER_YUPIX { "yupix" } else { "alice" };
            json!({"role": x["role"], "user": user(uid, name)})
        }).collect::<Vec<_>>());
    }
    if body["clear_description"] == true {
        t["description"] = Value::Null;
    }
    // 完了ステータスへの移動で completed_at を立てる
    if t["status_id"] == ST_DONE && t["completed_at"].is_null() {
        t["completed_at"] = json!(chrono::Utc::now().to_rfc3339());
    }
    t["updated_at"] = json!(chrono::Utc::now().to_rfc3339());
    Json(t.clone()).into_response()
}

async fn list_comments(
    State(m): State<Shared>,
    Path((_, _, _)): Path<(String, String, Uuid)>,
) -> Json<Value> {
    let m = m.lock().unwrap();
    Json(json!({"comments": m.comments}))
}

async fn create_comment(
    State(m): State<Shared>,
    Path((_, _, task_id)): Path<(String, String, Uuid)>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let mut m = m.lock().unwrap();
    let c = json!({
        "id": Uuid::new_v4(), "task_id": task_id,
        "body": body["body"], "user_id": USER_YUPIX,
        "parent_comment_id": body["parent_comment_id"],
        "created_at": chrono::Utc::now().to_rfc3339(),
        "updated_at": chrono::Utc::now().to_rfc3339(),
        "deleted_at": null,
    });
    m.comments.push(json!({
        "id": c["id"], "body": c["body"], "created_at": c["created_at"],
        "updated_at": c["updated_at"], "is_deleted": false,
        "user": {"id": USER_YUPIX, "name": "yupix", "avatar_url": null},
        "replies": [],
    }));
    Json(c).into_response()
}

// ---- notifications ----

#[derive(serde::Deserialize)]
struct NotifQuery {
    unread: Option<bool>,
    kind: Option<String>,
    limit: Option<u32>,
}

async fn list_notifications(
    State(m): State<Shared>,
    Query(q): Query<NotifQuery>,
) -> Json<Value> {
    let m = m.lock().unwrap();
    let mut items: Vec<Value> = m.notifications.clone();
    if q.unread == Some(true) {
        items.retain(|n| n["read_at"].is_null());
    }
    if let Some(k) = &q.kind {
        items.retain(|n| n["notification_type"].as_str().unwrap_or("").starts_with(k.as_str()));
    }
    if let Some(l) = q.limit {
        items.truncate(l as usize);
    }
    let unread = m
        .notifications
        .iter()
        .filter(|n| n["read_at"].is_null())
        .count();
    Json(json!({"unread_count": unread, "next_cursor": null, "notifications": items}))
}

async fn mark_notification_read(State(m): State<Shared>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let mut m = m.lock().unwrap();
    for n in m.notifications.iter_mut() {
        if n["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()) == Some(id) {
            n["read_at"] = json!(chrono::Utc::now().to_rfc3339());
        }
    }
    StatusCode::OK.into_response()
}

async fn mark_all_read(State(m): State<Shared>) -> impl IntoResponse {
    let mut m = m.lock().unwrap();
    for n in m.notifications.iter_mut() {
        if n["read_at"].is_null() {
            n["read_at"] = json!(chrono::Utc::now().to_rfc3339());
        }
    }
    StatusCode::OK.into_response()
}

// ---- devices / desktop auth ----

async fn list_devices(State(m): State<Shared>) -> Json<Value> {
    Json(json!({"devices": m.lock().unwrap().devices}))
}

async fn delete_device(State(m): State<Shared>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let mut m = m.lock().unwrap();
    m.devices
        .retain(|d| d["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()) != Some(id));
    StatusCode::NO_CONTENT.into_response()
}

async fn exchange_desktop_code() -> Json<Value> {
    Json(json!({
        "token": "kdt_mock_dev_token",
        "expires_at": "2027-09-23T00:00:00Z",
    }))
}

// ---- reviews ----

async fn reviewed_prs(State(m): State<Shared>) -> Json<Value> {
    let m = m.lock().unwrap();
    let open = m.findings.iter().filter(|f| f["state"] == "open").count() as i64;
    Json(json!([{
        "pr_number": 42, "pr_title": "feat: add mock API", "pr_author": "alice",
        "rounds": m.reviews.len() as i32, "blocking": open,
        "unresolved": open, "last_reviewed_at": "2026-09-22T11:00:00Z",
    }]))
}

async fn list_reviews(State(m): State<Shared>) -> Json<Value> {
    Json(json!(m.lock().unwrap().reviews))
}

async fn get_review(State(m): State<Shared>, Path((_, _, id)): Path<(String, String, Uuid)>) -> impl IntoResponse {
    let m = m.lock().unwrap();
    match m.reviews.iter().find(|r| {
        r["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()) == Some(id)
    }) {
        Some(r) => {
            let mut d = r.clone();
            d["findings"] = json!(m
                .findings
                .iter()
                .filter(|f| f["review_id"] == r["id"])
                .cloned()
                .collect::<Vec<_>>());
            Json(d).into_response()
        }
        None => err(StatusCode::NOT_FOUND, "review not found").into_response(),
    }
}

async fn review_summary(State(m): State<Shared>, Query(q): Query<std::collections::HashMap<String, String>>) -> impl IntoResponse {
    let m = m.lock().unwrap();
    let _pr = q.get("pr").and_then(|s| s.parse::<i64>().ok()).unwrap_or(42);
    let mut counts = Vec::new();
    for sev in ["high", "medium", "low", "nit"] {
        for st in ["open", "fixed", "deferred", "rejected"] {
            let n = m.findings.iter().filter(|f| {
                f["severity"] == sev && f["state"] == st
            }).count() as i64;
            if n > 0 {
                counts.push(json!({"severity": sev, "state": st, "count": n}));
            }
        }
    }
    let blocking = m.findings.iter().filter(|f| f["state"] == "open").count() as i64;
    Json(json!({
        "pr_number": 42, "repository": "koyori-app/task",
        "rounds": m.reviews.len() as i64, "counts": counts,
        "blocking": blocking, "owner_override_rejections": 0,
        "mergeable": blocking == 0,
        "latest_head_sha": "0123456789abcdef0123456789abcdef01234567",
        "cached_pr_head_sha": "0123456789abcdef0123456789abcdef01234567",
        "pr_head_checked_at": "2026-09-23T00:00:00Z",
        "gate": if blocking == 0 { "ready" } else { "blocked" },
    }))
    .into_response()
}

async fn list_findings(State(m): State<Shared>) -> Json<Value> {
    Json(json!(m.lock().unwrap().findings))
}

async fn update_finding(
    State(m): State<Shared>,
    Path((_, _, id)): Path<(String, String, Uuid)>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let mut m = m.lock().unwrap();
    let Some(f) = m.findings.iter_mut().find(|f| {
        f["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()) == Some(id)
    }) else {
        return err(StatusCode::NOT_FOUND, "finding not found").into_response();
    };
    let from = f["state"].clone();
    f["state"] = body["state"].clone();
    if let Some(tr) = f["transitions"].as_array_mut() {
        tr.push(json!({
            "id": Uuid::new_v4(), "actor": user(USER_YUPIX, "yupix"),
            "from_state": from, "to_state": body["state"],
            "note": body["note"], "created_at": chrono::Utc::now().to_rfc3339(),
        }));
    }
    f["updated_at"] = json!(chrono::Utc::now().to_rfc3339());
    Json(f.clone()).into_response()
}

async fn create_review(
    State(m): State<Shared>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let mut m = m.lock().unwrap();
    let id = Uuid::new_v4();
    let round = m.reviews.len() as i32 + 1;
    let r = json!({
        "id": id, "project_id": PROJECT,
        "pr_number": body["pr_number"], "pr_title": "feat: add mock API",
        "pr_author": "alice", "round": round,
        "head_sha": body["head_sha"],
        "summary": body["summary"].as_str().unwrap_or(""),
        "reviewer": user(USER_YUPIX, "yupix"), "reviewer_left_tenant": false,
        "finding_count": body["findings"].as_array().map(|a| a.len()).unwrap_or(0) as i64,
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    m.reviews.push(r);
    let mut detail = m.reviews.last().unwrap().clone();
    detail["findings"] = json!([]);
    Json(detail).into_response()
}

async fn log_request(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    eprintln!("{} {}", req.method(), req.uri());
    next.run(req).await
}

async fn fallback() -> impl IntoResponse {
    err(StatusCode::NOT_FOUND, "mock: unknown endpoint")
}

#[tokio::main]
async fn main() {
    let state: Shared = Arc::new(Mutex::new(Mock::seed()));
    let t = "/v1/tenants/{tenant}";
    let tp = format!("{t}/projects/{{project}}");
    // api crate は `base`（…/api）に `/v1/...` を継ぐので /api 配下にマウント。
    let v1 = Router::new()
        .route("/v1/desktop/auth/token", post(exchange_desktop_code))
        .route("/v1/users/me/devices", get(list_devices))
        .route("/v1/users/me/devices/{id}", delete(delete_device))
        .route("/v1/users/me/notifications", get(list_notifications))
        .route("/v1/users/me/notifications/{id}/read", patch(mark_notification_read))
        .route("/v1/users/me/notifications/read-all", patch(mark_all_read))
        .route("/v1/tenants", get(tenants))
        .route(&format!("{t}/projects"), get(projects))
        .route(&format!("{t}/users/me/personal-project"), get(personal_project))
        .route(&format!("{t}/users/me/tasks"), get(my_tasks))
        .route(&format!("{tp}/statuses"), get(statuses))
        .route(&format!("{tp}/assignable-users"), get(assignable_users))
        .route(&format!("{tp}/tasks"), get(list_tasks).post(create_task))
        .route(&format!("{tp}/tasks/{{task}}"), get(get_task).put(update_task))
        .route(&format!("{tp}/tasks/{{task}}/comments"), get(list_comments).post(create_comment))
        .route(&format!("{tp}/reviews/pull-requests"), get(reviewed_prs))
        .route(&format!("{tp}/reviews/summary"), get(review_summary))
        .route(&format!("{tp}/reviews"), get(list_reviews).post(create_review))
        .route(&format!("{tp}/reviews/{{review}}"), get(get_review))
        .route(&format!("{tp}/review-findings"), get(list_findings))
        .route(&format!("{tp}/review-findings/{{finding}}"), patch(update_finding))
        .layer(axum::middleware::from_fn(log_request))
        .with_state(state);
    // `api` クレートの Client は base（…/api まで）に `v1/...` を継ぐ。
    let app = Router::new().nest("/api", v1).fallback(fallback);

    let port = std::env::var("KOYORI_MOCK_PORT").unwrap_or_else(|_| "4199".into());
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    eprintln!("koyori-mock listening on http://{addr}  (api base: http://{addr}/api)");
    axum::serve(listener, app).await.unwrap();
}
