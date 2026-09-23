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
use base64::Engine;
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
            task(
                t1,
                1,
                "モック環境の動作確認",
                ST_TODO,
                "High",
                me.clone(),
                PROJECT,
            ),
            task(
                t2,
                2,
                "通知センターの表示確認",
                ST_DOING,
                "Medium",
                both,
                PROJECT,
            ),
            task(
                t3,
                3,
                "コメント投稿のテスト",
                ST_BACKLOG,
                "Low",
                json!([]),
                PROJECT,
            ),
            task(
                t4,
                4,
                "完了済みタスクの例",
                ST_DONE,
                "Trivial",
                me.clone(),
                PROJECT,
            ),
            task(
                Uuid::new_v4(),
                1,
                "別テナントのタスク",
                ST_TODO,
                "Medium",
                me,
                PROJECT2,
            ),
            task(
                Uuid::new_v4(),
                2,
                "OTHER プロジェクトの件",
                ST_DOING,
                "High",
                json!([]),
                PROJECT2,
            ),
        ];
        tasks[3]["completed_at"] = json!("2026-09-22T15:00:00Z");
        let today = chrono::Local::now().date_naive();
        tasks[0]["hard_deadline"] = json!(format!("{today}T09:00:00Z"));
        tasks[1]["hard_deadline"] =
            json!(format!("{}T09:00:00Z", today + chrono::Duration::days(2)));

        let mut comments = vec![
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
        for comment in &mut comments {
            comment["task_id"] = json!(t1);
        }

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
        let mut notifications = vec![
            n_task(
                &t1,
                "assigned",
                json!({"actor": "alice"}),
                json!({"type": "task", "task_id": t1}),
                false,
            ),
            n_task(
                &t2,
                "comment_added",
                json!({"actor": "alice"}),
                json!({"type": "task", "task_id": t2}),
                false,
            ),
            json!({
                "id": Uuid::new_v4(), "notification_type": "review.finding_fixed",
                "project": {"tenant_id": TENANT, "id": PROJECT, "key": "MOCK"},
                "task": null,
                "payload": {"actor": "alice", "pr_number": 42, "round": 1},
                "target": {"type": "review_finding", "review_id": REVIEW_1, "finding_id": "11111111-2222-3333-4444-555555555555"},
                "read_at": null, "created_at": "2026-09-23T09:00:00Z",
                "cursor": format!("mock-cursor-{}", Uuid::new_v4()),
            }),
            n_task(
                &t3,
                "mentioned",
                json!({"actor": "alice"}),
                json!({"type": "task", "task_id": t3}),
                true,
            ),
            n_task(
                &Uuid::nil(),
                "status_changed",
                json!({"actor": "alice"}),
                json!({"type": "task", "task_id": Uuid::nil()}),
                true,
            ),
        ];
        for notification in &mut notifications {
            notification["cursor"] = json!(notification_cursor(notification));
        }

        let f1 = Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
        let finding = |id: Uuid,
                       sev: &str,
                       st: &str,
                       title: &str,
                       file: &str,
                       line: i32,
                       actions: &[&str]| {
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
            finding(
                f1,
                "high",
                "open",
                "エラーハンドリングが不足",
                "src/main.rs",
                42,
                &["fixed", "deferred", "rejected"],
            ),
            finding(
                Uuid::new_v4(),
                "medium",
                "open",
                "命名が曖昧",
                "src/lib.rs",
                10,
                &["fixed", "deferred", "rejected"],
            ),
            finding(
                Uuid::new_v4(),
                "low",
                "fixed",
                "不要な clone",
                "src/util.rs",
                88,
                &["verified", "open"],
            ),
            finding(
                Uuid::new_v4(),
                "nit",
                "open",
                "コメントの typo",
                "README.md",
                3,
                &["fixed", "deferred", "rejected"],
            ),
        ];

        let review = |id: &str, round: i32| {
            json!({
                "id": id, "project_id": PROJECT, "pr_number": 42,
                "pr_title": "feat: add mock API", "pr_author": "alice",
                "round": round, "head_sha": "0123456789abcdef0123456789abcdef01234567",
                "summary": format!("R{round} の総評。"),
                "reviewer": user(USER_YUPIX, "yupix"), "reviewer_left_tenant": false,
                "finding_count": if round == 1 { 4 } else { 0 }, "created_at": "2026-09-22T11:00:00Z",
            })
        };

        Self {
            tasks,
            comments,
            notifications,
            findings,
            reviews: vec![review(REVIEW_1, 1), review(REVIEW_2, 2)],
            devices: vec![
                json!({"id": Uuid::new_v4(), "name": std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "desktop".into()), "last_used_at": chrono::Utc::now().to_rfc3339(), "expires_at": (chrono::Utc::now() + chrono::Duration::days(90)).to_rfc3339(), "created_at": "2026-09-01T00:00:00Z"}),
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

fn notification_key(item: &Value) -> (chrono::DateTime<chrono::Utc>, Uuid) {
    (
        item["created_at"].as_str().unwrap().parse().unwrap(),
        item["id"].as_str().unwrap().parse().unwrap(),
    )
}

fn notification_cursor(item: &Value) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        json!({
            "created_at": item["created_at"], "id": item["id"],
        })
        .to_string(),
    )
}

fn parse_notification_cursor(cursor: &str) -> Option<(chrono::DateTime<chrono::Utc>, Uuid)> {
    let value: Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(cursor)
            .ok()?,
    )
    .ok()?;
    Some((
        value["created_at"].as_str()?.parse().ok()?,
        value["id"].as_str()?.parse().ok()?,
    ))
}

fn push_notification(
    m: &mut Mock,
    project: &str,
    ty: &str,
    payload: Value,
    target: Value,
    task: Value,
) {
    let key = if project == PROJECT2 {
        "OTH"
    } else if project == PERSONAL {
        "PERSONAL"
    } else {
        "MOCK"
    };
    let mut item = json!({
        "id": Uuid::new_v4(), "notification_type": ty,
        "project": {"tenant_id": tenant_of_project(project), "id": project, "key": key},
        "task": task, "payload": payload, "target": target,
        "read_at": null, "created_at": chrono::Utc::now().to_rfc3339(),
    });
    item["cursor"] = json!(notification_cursor(&item));
    m.notifications.push(item);
}

// ---- handlers ----

async fn get_me() -> Json<Value> {
    Json(
        json!({"id": USER_YUPIX, "username": "yupix", "email": "yupix@example.test",
        "email_verified": true, "is_admin": false, "is_suspended": false,
        "totp_enabled": true, "has_password": true, "avatar_url": null, "bio": "Mock account"}),
    )
}

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
        v.insert(
            0,
            json!({"id": PROJECT, "tenant_id": TENANT, "key": "MOCK", "name": "Mock Project",
            "description": "", "icon_url": null, "icon_emoji": "📦",
            "is_personal": false, "personal_owner_id": null}),
        );
    } else {
        v.insert(
            0,
            json!({"id": PROJECT2, "tenant_id": TENANT2, "key": "OTH", "name": "Other Project",
            "description": "", "icon_url": null, "icon_emoji": "📁",
            "is_personal": false, "personal_owner_id": null}),
        );
    }
    Json(json!(v))
}

async fn personal_project(Path(tenant): Path<String>) -> Json<Value> {
    Json(
        json!({"id": PERSONAL, "tenant_id": tenant, "key": "PERSONAL", "name": "Personal",
        "description": "", "icon_url": null, "icon_emoji": null,
        "is_personal": true, "personal_owner_id": USER_YUPIX}),
    )
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
    Json(json!([
        user(USER_YUPIX, "yupix"),
        user(USER_ALICE, "alice")
    ]))
}

fn tenant_of_project(p: &str) -> &str {
    if p == PROJECT2 { TENANT2 } else { TENANT }
}

async fn list_tasks(
    State(m): State<Shared>,
    Path((_, p)): Path<(String, String)>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let m = m.lock().unwrap();
    let tasks: Vec<Value> = m
        .tasks
        .iter()
        .filter(|t| t["project_id"] == p)
        .filter(|t| query.get("status_id").is_none_or(|s| t["status_id"] == *s))
        .cloned()
        .collect();
    Json(json!({"tasks": tasks, "total": tasks.len(), "next_cursor": null}))
}

async fn search_tasks(
    State(m): State<Shared>,
    Path((_, project)): Path<(String, String)>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let m = m.lock().unwrap();
    let q = query.get("q").map(|s| s.to_lowercase()).unwrap_or_default();
    let tasks: Vec<_> = m
        .tasks
        .iter()
        .filter(|t| t["project_id"] == project)
        .filter(|t| {
            t["title"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase()
                .contains(&q)
                || t["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&q)
        })
        .map(|t| {
            json!({"id": t["id"], "seq_id": t["seq_id"], "title": t["title"],
            "highlight": t["description"].as_str().unwrap_or_default(), "score": 1.0})
        })
        .collect();
    Json(json!({"tasks": tasks, "total": tasks.len()}))
}

async fn my_tasks(State(m): State<Shared>, Path(tenant): Path<String>) -> Json<Value> {
    let m = m.lock().unwrap();
    let items: Vec<Value> = m
        .tasks
        .iter()
        .filter(|t| {
            tenant_of_project(t["project_id"].as_str().unwrap_or_default()) == tenant
                && t["assignees"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| x["user"]["id"] == USER_YUPIX))
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

async fn get_task(
    State(m): State<Shared>,
    Path((_, _, id)): Path<(String, String, Uuid)>,
) -> impl IntoResponse {
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
    for key in ["description", "soft_deadline", "hard_deadline"] {
        if !body[key].is_null() {
            t[key] = body[key].clone();
        }
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
    for key in [
        "title",
        "description",
        "priority",
        "progress_pct",
        "status_id",
        "soft_deadline",
        "hard_deadline",
        "estimated_minutes",
        "is_archived",
        "milestone_id",
        "sprint_id",
        "parent_task_id",
    ] {
        if !body[key].is_null() {
            t[key] = body[key].clone();
        }
    }
    if let Some(a) = body["assignees"].as_array() {
        t["assignees"] = json!(
            a.iter()
                .map(|x| {
                    let uid = x["user_id"].as_str().unwrap_or(USER_YUPIX);
                    let name = if uid == USER_YUPIX { "yupix" } else { "alice" };
                    json!({"role": x["role"], "user": user(uid, name)})
                })
                .collect::<Vec<_>>()
        );
    }
    if body["clear_description"] == true {
        t["description"] = Value::Null;
    }
    for key in ["soft_deadline", "hard_deadline"] {
        if body[format!("clear_{key}")] == true {
            t[key] = Value::Null;
        }
    }
    // 完了ステータスへの移動で completed_at を立てる
    if t["status_id"] == ST_DONE && t["completed_at"].is_null() {
        t["completed_at"] = json!(chrono::Utc::now().to_rfc3339());
    }
    if t["status_id"] != ST_DONE {
        t["completed_at"] = Value::Null;
    }
    t["updated_at"] = json!(chrono::Utc::now().to_rfc3339());
    Json(t.clone()).into_response()
}

async fn list_comments(
    State(m): State<Shared>,
    Path((_, _, task)): Path<(String, String, Uuid)>,
) -> Json<Value> {
    let m = m.lock().unwrap();
    Json(
        json!({"comments": m.comments.iter().filter(|c| c["task_id"] == json!(task)).collect::<Vec<_>>() }),
    )
}

async fn create_comment(
    State(m): State<Shared>,
    Path((_, _, task_id)): Path<(String, String, Uuid)>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let mut m = m.lock().unwrap();
    let Some(task_index) = find_task(&m, task_id) else {
        return err(StatusCode::NOT_FOUND, "task not found").into_response();
    };
    if body["body"].as_str().is_none_or(|b| b.trim().is_empty()) {
        return err(StatusCode::BAD_REQUEST, "Comment body is required").into_response();
    }
    let c = json!({
        "id": Uuid::new_v4(), "task_id": task_id,
        "body": body["body"], "user_id": USER_YUPIX,
        "parent_comment_id": body["parent_comment_id"],
        "created_at": chrono::Utc::now().to_rfc3339(),
        "updated_at": chrono::Utc::now().to_rfc3339(),
        "deleted_at": null,
    });
    m.comments.push(json!({
        "task_id": task_id,
        "id": c["id"], "body": c["body"], "created_at": c["created_at"],
        "updated_at": c["updated_at"], "is_deleted": false,
        "user": {"id": USER_YUPIX, "name": "yupix", "avatar_url": null},
        "replies": [],
    }));
    let task = m.tasks[task_index].clone();
    push_notification(
        &mut m,
        task["project_id"].as_str().unwrap(),
        "comment_added",
        json!({"actor": "yupix"}),
        json!({"type": "task", "task_id": task_id}),
        json!({"id": task_id, "seq_id": task["seq_id"], "title": task["title"]}),
    );
    Json(c).into_response()
}

// ---- notifications ----

#[derive(Default, serde::Deserialize)]
struct NotifQuery {
    unread: Option<bool>,
    kind: Option<String>,
    limit: Option<u32>,
    cursor: Option<String>,
    after: Option<String>,
}

async fn list_notifications(
    State(m): State<Shared>,
    Query(q): Query<NotifQuery>,
) -> axum::response::Response {
    let m = m.lock().unwrap();
    if q.cursor.is_some() && q.after.is_some() {
        return err(
            StatusCode::BAD_REQUEST,
            "cursor and after are mutually exclusive",
        )
        .into_response();
    }
    let mut items: Vec<Value> = m.notifications.clone();
    if q.unread == Some(true) {
        items.retain(|n| n["read_at"].is_null());
    }
    if let Some(k) = &q.kind {
        items.retain(|n| {
            let is_review = n["notification_type"]
                .as_str()
                .unwrap_or("")
                .starts_with("review.");
            if k == "review" { is_review } else { !is_review }
        });
    }
    items.sort_by_key(notification_key);
    if let Some(cursor) = q.after.as_ref().or(q.cursor.as_ref()) {
        let Some(bound) = parse_notification_cursor(cursor) else {
            return err(StatusCode::BAD_REQUEST, "invalid notification cursor").into_response();
        };
        items.retain(|item| {
            if q.after.is_some() {
                notification_key(item) > bound
            } else {
                notification_key(item) < bound
            }
        });
    }
    if q.after.is_none() {
        items.reverse();
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 100) as usize;
    let has_more = items.len() > limit;
    items.truncate(limit);
    let next_cursor = if has_more {
        items.last().map(notification_cursor)
    } else {
        None
    };
    let unread = m
        .notifications
        .iter()
        .filter(|n| n["read_at"].is_null())
        .count();
    Json(json!({"unread_count": unread, "next_cursor": next_cursor, "notifications": items}))
        .into_response()
}

async fn mark_notification_read(
    State(m): State<Shared>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
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
        "expires_at": (chrono::Utc::now() + chrono::Duration::days(90)).to_rfc3339(),
    }))
}

// ---- reviews ----

fn belongs_to_project(m: &Mock, finding: &Value, project: &str) -> bool {
    m.reviews
        .iter()
        .any(|r| r["id"] == finding["review_id"] && r["project_id"] == project)
}

fn blocks(f: &Value) -> bool {
    (f["severity"] == "high" || f["severity"] == "medium")
        && (f["state"] == "open" || f["state"] == "fixed")
}

async fn reviewed_prs(
    State(m): State<Shared>,
    Path((_, project)): Path<(String, String)>,
) -> Json<Value> {
    let m = m.lock().unwrap();
    let mut prs = std::collections::BTreeMap::new();
    for review in m.reviews.iter().filter(|r| r["project_id"] == project) {
        let number = review["pr_number"].as_i64().unwrap();
        let row = prs.entry(number).or_insert_with(|| json!({
            "pr_number": number, "pr_title": review["pr_title"], "pr_author": review["pr_author"],
            "rounds": 0, "blocking": 0, "unresolved": 0, "last_reviewed_at": review["created_at"],
        }));
        row["rounds"] = json!(row["rounds"].as_i64().unwrap() + 1);
        row["last_reviewed_at"] = review["created_at"].clone();
    }
    for f in m
        .findings
        .iter()
        .filter(|f| belongs_to_project(&m, f, &project))
    {
        if let Some(row) = prs.get_mut(&f["pr_number"].as_i64().unwrap()) {
            if f["state"] == "open" || f["state"] == "fixed" {
                row["unresolved"] = json!(row["unresolved"].as_i64().unwrap() + 1);
            }
            if blocks(f) {
                row["blocking"] = json!(row["blocking"].as_i64().unwrap() + 1);
            }
        }
    }
    Json(json!(prs.into_values().collect::<Vec<_>>()))
}

async fn list_reviews(
    State(m): State<Shared>,
    Path((_, project)): Path<(String, String)>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let m = m.lock().unwrap();
    Json(json!(
        m.reviews
            .iter()
            .filter(|r| r["project_id"] == project)
            .filter(|r| query.get("pr").is_none_or(|p| r["pr_number"]
                .as_i64()
                .map(|n| n.to_string())
                .as_ref()
                == Some(p)))
            .collect::<Vec<_>>()
    ))
}

async fn get_review(
    State(m): State<Shared>,
    Path((_, _, id)): Path<(String, String, Uuid)>,
) -> impl IntoResponse {
    let m = m.lock().unwrap();
    match m
        .reviews
        .iter()
        .find(|r| r["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()) == Some(id))
    {
        Some(r) => {
            let mut d = r.clone();
            d["findings"] = json!(
                m.findings
                    .iter()
                    .filter(|f| f["review_id"] == r["id"])
                    .cloned()
                    .collect::<Vec<_>>()
            );
            Json(d).into_response()
        }
        None => err(StatusCode::NOT_FOUND, "review not found").into_response(),
    }
}

async fn review_summary(
    State(m): State<Shared>,
    Path((_, project)): Path<(String, String)>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let m = m.lock().unwrap();
    let pr = q
        .get("pr")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(42);
    let findings: Vec<_> = m
        .findings
        .iter()
        .filter(|f| belongs_to_project(&m, f, &project) && f["pr_number"] == pr)
        .collect();
    let reviews: Vec<_> = m
        .reviews
        .iter()
        .filter(|r| r["project_id"] == project && r["pr_number"] == pr)
        .collect();
    let mut counts = Vec::new();
    for sev in ["high", "medium", "low", "nit"] {
        for st in ["open", "fixed", "verified", "deferred", "rejected"] {
            let n = findings
                .iter()
                .filter(|f| f["severity"] == sev && f["state"] == st)
                .count() as i64;
            if n > 0 {
                counts.push(json!({"severity": sev, "state": st, "count": n}));
            }
        }
    }
    let blocking = findings.iter().filter(|f| blocks(f)).count() as i64;
    Json(json!({
        "pr_number": pr, "repository": "koyori-app/task",
        "rounds": reviews.len() as i64, "counts": counts,
        "blocking": blocking, "owner_override_rejections": 0,
        "mergeable": blocking == 0,
        "latest_head_sha": reviews.last().map(|r| r["head_sha"].clone()),
        "cached_pr_head_sha": reviews.last().map(|r| r["head_sha"].clone()),
        "pr_head_checked_at": "2026-09-23T00:00:00Z",
        "gate": if reviews.is_empty() { "unreviewed" } else if blocking == 0 { "ready" } else { "blocked" },
    }))
    .into_response()
}

async fn list_findings(
    State(m): State<Shared>,
    Path((_, project)): Path<(String, String)>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let m = m.lock().unwrap();
    Json(json!(
        m.findings
            .iter()
            .filter(|f| belongs_to_project(&m, f, &project))
            .filter(|f| q.get("pr").is_none_or(|p| f["pr_number"]
                .as_i64()
                .map(|n| n.to_string())
                .as_ref()
                == Some(p)))
            .filter(|f| q.get("state").is_none_or(|s| f["state"] == *s))
            .filter(|f| q.get("severity").is_none_or(|s| f["severity"] == *s))
            .collect::<Vec<_>>()
    ))
}

async fn update_finding(
    State(m): State<Shared>,
    Path((_, project, id)): Path<(String, String, Uuid)>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let mut m = m.lock().unwrap();
    let Some(f) = m
        .findings
        .iter_mut()
        .find(|f| f["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()) == Some(id))
    else {
        return err(StatusCode::NOT_FOUND, "finding not found").into_response();
    };
    if !f["available_actions"]
        .as_array()
        .is_some_and(|a| a.contains(&body["state"]))
    {
        return err(
            StatusCode::CONFLICT,
            "This finding action is no longer available. Refresh and try again.",
        )
        .into_response();
    }
    let from = f["state"].clone();
    f["state"] = body["state"].clone();
    f["available_actions"] = match body["state"].as_str().unwrap_or_default() {
        "open" => json!(["fixed", "deferred", "rejected"]),
        "fixed" => json!(["verified", "open"]),
        "deferred" | "rejected" => json!(["open"]),
        _ => json!([]),
    };
    if let Some(tr) = f["transitions"].as_array_mut() {
        tr.push(json!({
            "id": Uuid::new_v4(), "actor": user(USER_YUPIX, "yupix"),
            "from_state": from, "to_state": body["state"],
            "note": body["note"], "created_at": chrono::Utc::now().to_rfc3339(),
        }));
    }
    f["updated_at"] = json!(chrono::Utc::now().to_rfc3339());
    let result = f.clone();
    let ty = match body["state"].as_str().unwrap_or_default() {
        "fixed" => Some("review.finding_fixed"),
        "verified" => Some("review.finding_verified"),
        "open" => Some("review.finding_reopened"),
        "deferred" => Some("review.finding_deferred"),
        _ => None,
    };
    if let Some(ty) = ty {
        push_notification(
            &mut m,
            &project,
            ty,
            json!({"actor": "yupix", "pr_number": result["pr_number"], "round": result["round"]}),
            json!({"type": "review_finding", "review_id": result["review_id"], "finding_id": id}),
            Value::Null,
        );
    }
    Json(result).into_response()
}

async fn create_review(
    State(m): State<Shared>,
    Path((_, project)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let mut m = m.lock().unwrap();
    if body["head_sha"].as_str().is_none_or(|s| {
        s.len() != 40
            || !s
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    }) {
        return err(
            StatusCode::BAD_REQUEST,
            "Head SHA must be 40 lowercase hexadecimal characters",
        )
        .into_response();
    }
    if body["pr_number"].as_i64().is_none_or(|n| n <= 0) {
        return err(
            StatusCode::BAD_REQUEST,
            "Pull request number must be positive",
        )
        .into_response();
    }
    let id = Uuid::new_v4();
    let round = m
        .reviews
        .iter()
        .filter(|r| r["project_id"] == project && r["pr_number"] == body["pr_number"])
        .count() as i32
        + 1;
    let now = chrono::Utc::now().to_rfc3339();
    let findings: Vec<_> = body["findings"].as_array().into_iter().flatten().map(|f| json!({
        "id": Uuid::new_v4(), "review_id": id, "pr_number": body["pr_number"], "round": round,
        "severity": f["severity"], "title": f["title"], "body": f["body"], "file": f["file"], "line": f["line"],
        "state": "open", "fixed_by": null, "deferred_task_id": null,
        "transitions": [{"id": Uuid::new_v4(), "actor": user(USER_YUPIX, "yupix"), "from_state": null, "to_state": "open", "note": null, "created_at": now}],
        "available_actions": ["fixed", "deferred", "rejected"], "created_at": now, "updated_at": now,
    })).collect();
    let r = json!({
        "id": id, "project_id": project,
        "pr_number": body["pr_number"], "pr_title": "feat: add mock API",
        "pr_author": "alice", "round": round,
        "head_sha": body["head_sha"],
        "summary": body["summary"].as_str().unwrap_or(""),
        "reviewer": user(USER_YUPIX, "yupix"), "reviewer_left_tenant": false,
        "finding_count": body["findings"].as_array().map(|a| a.len()).unwrap_or(0) as i64,
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    m.reviews.push(r.clone());
    m.findings.extend(findings.clone());
    push_notification(
        &mut m,
        &project,
        "review.round_created",
        json!({"actor": "yupix", "pr_number": body["pr_number"], "round": round, "finding_count": findings.len(), "blocking_count": findings.iter().filter(|f| blocks(f)).count()}),
        json!({"type": "review", "review_id": id}),
        Value::Null,
    );
    let mut detail = r;
    detail["findings"] = json!(findings);
    (StatusCode::CREATED, Json(detail)).into_response()
}

async fn log_request(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    eprintln!("{} {}", req.method(), req.uri());
    if req
        .headers()
        .get("authorization")
        .is_some_and(|value| value == "Bearer expired")
    {
        return err(
            StatusCode::UNAUTHORIZED,
            "Device token expired. Sign in again.",
        )
        .into_response();
    }
    next.run(req).await
}

async fn fallback() -> impl IntoResponse {
    err(StatusCode::NOT_FOUND, "mock: unknown endpoint")
}

fn router(state: Shared) -> Router {
    let t = "/v1/tenants/{tenant}";
    let tp = format!("{t}/projects/{{project}}");
    // api crate は `base`（…/api）に `/v1/...` を継ぐので /api 配下にマウント。
    let v1 = Router::new()
        .route("/v1/auth/me", get(get_me))
        .route("/v1/desktop/auth/token", post(exchange_desktop_code))
        .route("/v1/users/me/devices", get(list_devices))
        .route("/v1/users/me/devices/{id}", delete(delete_device))
        .route("/v1/users/me/notifications", get(list_notifications))
        .route(
            "/v1/users/me/notifications/{id}/read",
            patch(mark_notification_read),
        )
        .route("/v1/users/me/notifications/read-all", patch(mark_all_read))
        .route("/v1/tenants", get(tenants))
        .route(&format!("{t}/projects"), get(projects))
        .route(
            &format!("{t}/users/me/personal-project"),
            get(personal_project),
        )
        .route(&format!("{t}/users/me/tasks"), get(my_tasks))
        .route(&format!("{tp}/statuses"), get(statuses))
        .route(&format!("{tp}/assignable-users"), get(assignable_users))
        .route(&format!("{tp}/tasks"), get(list_tasks).post(create_task))
        .route(&format!("{tp}/tasks/search"), get(search_tasks))
        .route(
            &format!("{tp}/tasks/{{task}}"),
            get(get_task).put(update_task),
        )
        .route(
            &format!("{tp}/tasks/{{task}}/comments"),
            get(list_comments).post(create_comment),
        )
        .route(&format!("{tp}/reviews/pull-requests"), get(reviewed_prs))
        .route(&format!("{tp}/reviews/summary"), get(review_summary))
        .route(
            &format!("{tp}/reviews"),
            get(list_reviews).post(create_review),
        )
        .route(&format!("{tp}/reviews/{{review}}"), get(get_review))
        .route(&format!("{tp}/review-findings"), get(list_findings))
        .route(
            &format!("{tp}/review-findings/{{finding}}"),
            patch(update_finding),
        )
        .layer(axum::middleware::from_fn(log_request))
        .with_state(state);
    // `api` クレートの Client は base（…/api まで）に `v1/...` を継ぐ。
    Router::new().nest("/api", v1).fallback(fallback)
}

#[tokio::main]
async fn main() {
    let app = router(Arc::new(Mutex::new(Mock::seed())));

    let port = std::env::var("KOYORI_MOCK_PORT").unwrap_or_else(|_| "4199".into());
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    eprintln!("koyori-mock listening on http://{addr}  (api base: http://{addr}/api)");
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn client() -> (api::Client, tokio::task::JoinHandle<()>, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/api", listener.local_addr().unwrap());
        let app = router(Arc::new(Mutex::new(Mock::seed())));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (api::Client::new(&base, "dev").unwrap(), server, base)
    }

    #[tokio::test]
    async fn mock_contracts_preserve_changes_and_scope_comments() {
        let (client, server, _) = client().await;
        let tenant = TENANT.parse().unwrap();
        let project = PROJECT.parse().unwrap();
        assert_eq!(client.get_me().await.unwrap().username, "yupix");
        let tasks = client
            .list_tasks(tenant, project, &Default::default())
            .await
            .unwrap()
            .tasks;
        let first = tasks[0].id;
        let second = tasks[1].id;
        let before = client
            .list_comments(tenant, project, first)
            .await
            .unwrap()
            .comments
            .len();
        let second_before = client
            .list_comments(tenant, project, second)
            .await
            .unwrap()
            .comments
            .len();
        let comment = serde_json::from_value(json!({"body": "Contract test comment"})).unwrap();
        client
            .create_comment(tenant, project, first, &comment)
            .await
            .unwrap();
        assert_eq!(
            client
                .list_comments(tenant, project, first)
                .await
                .unwrap()
                .comments
                .len(),
            before + 1
        );
        assert_eq!(
            client
                .list_comments(tenant, project, second)
                .await
                .unwrap()
                .comments
                .len(),
            second_before
        );
        let search = client
            .search_tasks(tenant, project, "モック環境")
            .await
            .unwrap();
        assert_eq!(search.tasks.len(), 1);
        assert_eq!(search.tasks[0].id, first);

        let body = serde_json::from_value(json!({"pr_number": 73, "head_sha": "a".repeat(40), "summary": "Review summary", "findings": [{
            "severity": "high", "title": "Important finding", "body": "Details", "file": "src/lib.rs", "line": 12
        }]})).unwrap();
        let review = client.create_review(tenant, project, &body).await.unwrap();
        assert_eq!(review.round, 1);
        assert_eq!(review.findings.len(), 1);
        let findings = client
            .list_review_findings(
                tenant,
                project,
                &api::FindingsQuery {
                    pr: Some(73),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(
            client
                .get_review_summary(tenant, project, 73, None)
                .await
                .unwrap()
                .gate,
            Some(api::spec::Gate::Blocked)
        );
        let fixed = client
            .update_finding_state(
                tenant,
                project,
                findings[0].id,
                api::types::FindingState::Fixed,
                None,
            )
            .await
            .unwrap();
        assert!(
            fixed
                .available_actions
                .contains(&api::types::FindingState::Verified)
        );
        let verified = client
            .update_finding_state(
                tenant,
                project,
                fixed.id,
                api::types::FindingState::Verified,
                Some("Confirmed".into()),
            )
            .await
            .unwrap();
        assert!(verified.available_actions.is_empty());
        assert_eq!(verified.transitions.len(), 3);
        assert!(matches!(
            client
                .update_finding_state(
                    tenant,
                    project,
                    fixed.id,
                    api::types::FindingState::Open,
                    None
                )
                .await,
            Err(api::ApiError::Conflict { .. })
        ));
        assert_eq!(
            client
                .get_review_summary(tenant, project, 73, None)
                .await
                .unwrap()
                .gate,
            Some(api::spec::Gate::Ready)
        );
        server.abort();
    }

    #[tokio::test]
    async fn notification_paging_and_catch_up_include_read_items() {
        let (client, server, base) = client().await;
        let first = client
            .list_notifications(&api::NotificationsQuery {
                limit: Some(2),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(first.notifications.len(), 2);
        assert!(first.notifications[0].created_at >= first.notifications[1].created_at);
        let older = client
            .list_notifications(&api::NotificationsQuery {
                cursor: first.next_cursor,
                limit: Some(2),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(
            older
                .notifications
                .iter()
                .all(|n| first.notifications.iter().all(|f| n.id != f.id))
        );
        let tasks = client
            .list_notifications(&api::NotificationsQuery {
                kind: Some(api::spec::NotificationKind::Task),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(!tasks.notifications.is_empty());
        assert!(
            tasks
                .notifications
                .iter()
                .all(|n| !n.notification_type.starts_with("review."))
        );
        client.mark_all_notifications_read().await.unwrap();
        let after = older.notifications.last().unwrap().cursor.clone();
        let catchup = client
            .list_notifications(&api::NotificationsQuery {
                after,
                limit: Some(100),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(catchup.notifications.iter().all(|n| n.read_at.is_some()));
        assert!(
            catchup
                .notifications
                .windows(2)
                .all(|pair| (pair[0].created_at, pair[0].id) < (pair[1].created_at, pair[1].id))
        );
        let expired = api::Client::new(&base, "expired").unwrap();
        let clone = expired.clone();
        assert!(matches!(
            expired.get_me().await,
            Err(api::ApiError::Unauthorized)
        ));
        assert!(clone.is_unauthorized());
        server.abort();
    }
}
