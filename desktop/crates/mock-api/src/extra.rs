//! MVP 以降のエンドポイント（プロジェクト管理・タスクの付帯情報・ダッシュボード等）。
//! 形は openapi.json の応答型に合わせ、状態は `Mock::extra` にメモリで持つ。

use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{MethodRouter, get, post, put},
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    Mock, PERSONAL, PROJECT, PROJECT2, ST_BACKLOG, ST_DOING, ST_DONE, ST_TODO, Shared, TENANT,
    TENANT2, USER_ALICE, USER_YUPIX, err, find_task, rfc3339, status, user,
};

type Params = Path<HashMap<String, String>>;

fn now() -> String {
    rfc3339(chrono::Utc::now())
}

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

fn not_found() -> Response {
    err(StatusCode::NOT_FOUND, "not found").into_response()
}

fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

fn name_of(uid: &str) -> &'static str {
    if uid == USER_ALICE { "alice" } else { "yupix" }
}

/// 初期データ。`Mock::seed` の最後に呼ぶ。
pub(crate) fn seed(m: &mut Mock) {
    let project = |id: &str, tenant: &str, key: &str, name: &str, emoji: Value, personal: bool| {
        json!({"id": id, "tenant_id": tenant, "key": key, "name": name,
            "description": "", "icon_url": null, "icon_emoji": emoji,
            "is_personal": personal,
            "personal_owner_id": if personal { json!(USER_YUPIX) } else { Value::Null }})
    };
    m.projects = vec![
        project(PROJECT, TENANT, "MOCK", "Mock Project", json!("📦"), false),
        project(
            PROJECT2,
            TENANT2,
            "OTH",
            "Other Project",
            json!("📁"),
            false,
        ),
        project(PERSONAL, TENANT, "PERSONAL", "Personal", Value::Null, true),
    ];
    let start = today() - chrono::Duration::days(3);
    let sprint = json!({
        "id": Uuid::new_v4(), "project_id": PROJECT, "name": "Sprint 1",
        "goal": "モック API の確認", "status": "active",
        "start_date": start.to_string(), "end_date": (start + chrono::Duration::days(13)).to_string(),
        "created_by": USER_YUPIX, "created_at": "2026-09-01T00:00:00Z", "updated_at": "2026-09-01T00:00:00Z",
    });
    let milestone = json!({
        "id": Uuid::new_v4(), "project_id": PROJECT, "name": "v1.0",
        "description": "最初のリリース", "due_date": (today() + chrono::Duration::days(20)).to_string(),
        "created_by": USER_YUPIX, "created_at": "2026-09-01T00:00:00Z", "updated_at": "2026-09-01T00:00:00Z",
    });
    if let Some(first) = m.tasks.first_mut() {
        first["sprint_id"] = sprint["id"].clone();
        first["milestone_id"] = milestone["id"].clone();
    }
    m.extra.insert(format!("sprints:{PROJECT}"), vec![sprint]);
    m.extra
        .insert(format!("milestones:{PROJECT}"), vec![milestone]);
    m.extra.insert(
        format!("custom-fields:{PROJECT}"),
        vec![json!({
            "id": Uuid::new_v4(), "project_id": PROJECT, "name": "見積もり根拠",
            "field_type": "text", "is_required": false, "options": null, "position": 0,
            "created_at": "2026-09-01T00:00:00Z",
        })],
    );
    for tenant in [TENANT, TENANT2] {
        m.extra.insert(
            format!("tenant-members:{tenant}"),
            [(USER_YUPIX, "Admin"), (USER_ALICE, "Member")]
                .iter()
                .map(|(uid, role)| {
                    json!({"id": Uuid::new_v4(), "tenant_id": tenant, "user_id": uid,
                        "role": role, "user": user(uid, name_of(uid))})
                })
                .collect(),
        );
    }
    for p in [PROJECT, PROJECT2, PERSONAL] {
        m.extra.insert(
            format!("project-members:{p}"),
            vec![
                json!({"id": Uuid::new_v4(), "project_id": p, "user_id": USER_YUPIX,
                "role": "Admin", "user": user(USER_YUPIX, "yupix")}),
            ],
        );
    }
    m.profile = json!({"id": USER_YUPIX, "username": "yupix", "email": "yupix@example.test",
        "email_verified": true, "is_admin": false, "is_suspended": false,
        "totp_enabled": true, "has_password": true, "avatar_url": null, "bio": "Mock account"});
}

// ---- generic collections ----

/// 単純な CRUD で済む資源。`scope` は親を表すパス引数名。
struct Kind {
    name: &'static str,
    scope: &'static str,
    /// 一覧を `{wrap: [...]}` で返す型（`CustomFieldListResponse` など）。
    wrap: Option<&'static str>,
    /// 作成時に足す既定値。
    defaults: fn(&HashMap<String, String>) -> Value,
}

fn key(kind: &Kind, p: &HashMap<String, String>) -> String {
    format!(
        "{}:{}",
        kind.name,
        p.get(kind.scope).cloned().unwrap_or_default()
    )
}

fn wrap(kind: &Kind, items: Vec<Value>) -> Value {
    match kind.wrap {
        Some(field) => json!({ field: items }),
        None => json!(items),
    }
}

fn collection(kind: &'static Kind) -> MethodRouter<Shared> {
    get(move |State(m): State<Shared>, Path(p): Params| async move {
        let m = m.lock().unwrap();
        Json(wrap(
            kind,
            m.extra.get(&key(kind, &p)).cloned().unwrap_or_default(),
        ))
    })
    .post(
        move |State(m): State<Shared>, Path(p): Params, Json(body): Json<Value>| async move {
            let mut m = m.lock().unwrap();
            let mut item = (kind.defaults)(&p);
            merge(&mut item, &body);
            item["id"] = json!(Uuid::new_v4());
            item["created_at"] = json!(now());
            item["updated_at"] = json!(now());
            m.extra.entry(key(kind, &p)).or_default().push(item.clone());
            (StatusCode::CREATED, Json(item))
        },
    )
}

/// 1 件の GET。詳細を計算して返す資源（milestone / sprint）は別のハンドラにする。
fn item_get(kind: &'static Kind, id_param: &'static str) -> MethodRouter<Shared> {
    get(move |State(m): State<Shared>, Path(p): Params| async move {
        let m = m.lock().unwrap();
        let id = p.get(id_param).cloned().unwrap_or_default();
        m.extra
            .get(&key(kind, &p))
            .and_then(|items| items.iter().find(|x| x["id"] == id))
            .map(|x| Json(x.clone()).into_response())
            .unwrap_or_else(not_found)
    })
}

/// 1 件の更新（PUT / PATCH）と削除。
fn item(kind: &'static Kind, id_param: &'static str) -> MethodRouter<Shared> {
    let update = move |State(m): State<Shared>, Path(p): Params, Json(body): Json<Value>| async move {
        let mut m = m.lock().unwrap();
        let id = p.get(id_param).cloned().unwrap_or_default();
        let Some(item) = m
            .extra
            .get_mut(&key(kind, &p))
            .and_then(|items| items.iter_mut().find(|x| x["id"] == id))
        else {
            return not_found();
        };
        merge(item, &body);
        item["updated_at"] = json!(now());
        Json(item.clone()).into_response()
    };
    put(update)
        .patch(update)
        .delete(move |State(m): State<Shared>, Path(p): Params| async move {
            let mut m = m.lock().unwrap();
            let id = p.get(id_param).cloned().unwrap_or_default();
            let Some(items) = m.extra.get_mut(&key(kind, &p)) else {
                return not_found();
            };
            let before = items.len();
            items.retain(|x| x["id"] != id);
            if items.len() == before {
                return not_found();
            }
            no_content()
        })
}

/// `body` の値を上書きする。`clear_<field>: true` はその欄を null にする。
fn merge(target: &mut Value, body: &Value) {
    let Some(fields) = body.as_object() else {
        return;
    };
    for (k, v) in fields {
        if let Some(field) = k.strip_prefix("clear_") {
            if v == true {
                target[field] = Value::Null;
            }
        } else if !v.is_null() {
            target[k] = v.clone();
        }
    }
}

static MILESTONES: Kind = Kind {
    name: "milestones",
    scope: "project",
    wrap: None,
    defaults: |p| json!({"project_id": p["project"], "created_by": USER_YUPIX, "description": null}),
};
static SPRINTS: Kind = Kind {
    name: "sprints",
    scope: "project",
    wrap: None,
    defaults: |p| json!({"project_id": p["project"], "created_by": USER_YUPIX, "status": "planning", "goal": null}),
};
static CUSTOM_FIELDS: Kind = Kind {
    name: "custom-fields",
    scope: "project",
    wrap: Some("fields"),
    defaults: |p| json!({"project_id": p["project"], "is_required": false, "options": null, "position": 0}),
};
static TASK_VIEWS: Kind = Kind {
    name: "task-views",
    scope: "project",
    wrap: Some("views"),
    defaults: |p| {
        json!({"project_id": p["project"], "created_by": USER_YUPIX, "is_shared": false,
            "filters": {}, "sort": {}, "view_type": "list"})
    },
};
static TIME_LOGS: Kind = Kind {
    name: "time-logs",
    scope: "task",
    wrap: None,
    defaults: |p| json!({"task_id": p["task"], "user_id": USER_YUPIX, "note": null}),
};

// ---- account / tenants ----

async fn get_me(State(m): State<Shared>) -> Json<Value> {
    Json(m.lock().unwrap().profile.clone())
}

async fn update_me(State(m): State<Shared>, Json(body): Json<Value>) -> Response {
    if body["username"]
        .as_str()
        .is_some_and(|u| u.trim().is_empty())
    {
        return err(StatusCode::BAD_REQUEST, "username must not be empty").into_response();
    }
    let mut m = m.lock().unwrap();
    merge(&mut m.profile, &body);
    Json(m.profile.clone()).into_response()
}

fn tenant_value(id: &str, overrides: Option<&Value>) -> Value {
    let mut v = if id == TENANT {
        json!({"id": TENANT, "display_id": "mock", "name": "Mock Tenant", "description": "ローカルモック",
            "icon_url": "", "icon_emoji": "🧪", "owner_id": USER_YUPIX, "require_2fa": false,
            "drive_quota_bytes": null})
    } else {
        json!({"id": TENANT2, "display_id": "other", "name": "Other Tenant", "description": "2つ目のテナント",
            "icon_url": "", "icon_emoji": "🏢", "owner_id": USER_ALICE, "require_2fa": false,
            "drive_quota_bytes": null})
    };
    if let Some(o) = overrides {
        merge(&mut v, o);
    }
    v
}

pub(crate) fn tenant_override(m: &Mock, id: &str) -> Value {
    tenant_value(id, m.tenant_overrides.get(id))
}

async fn get_tenant(State(m): State<Shared>, Path(p): Params) -> Response {
    let id = &p["tenant"];
    if id != TENANT && id != TENANT2 {
        return not_found();
    }
    Json(tenant_override(&m.lock().unwrap(), id)).into_response()
}

async fn update_tenant(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    let id = p["tenant"].clone();
    if id != TENANT {
        // Other Tenant では Member なので変更できない。
        return err(StatusCode::FORBIDDEN, "only admins can update the tenant").into_response();
    }
    let mut m = m.lock().unwrap();
    let entry = m
        .tenant_overrides
        .entry(id.clone())
        .or_insert_with(|| json!({}));
    merge(entry, &body);
    Json(tenant_override(&m, &id)).into_response()
}

fn members_router(
    scope: &'static str,
    prefix: &'static str,
) -> (MethodRouter<Shared>, MethodRouter<Shared>) {
    let list = get(move |State(m): State<Shared>, Path(p): Params| async move {
        let m = m.lock().unwrap();
        Json(json!(m.extra.get(&format!("{prefix}:{}", p[scope])).cloned().unwrap_or_default()))
    })
    .post(
        move |State(m): State<Shared>, Path(p): Params, Json(body): Json<Value>| async move {
            let mut m = m.lock().unwrap();
            let uid = body["user_id"].as_str().unwrap_or_default().to_string();
            let list = m.extra.entry(format!("{prefix}:{}", p[scope])).or_default();
            if list.iter().any(|x| x["user_id"] == uid) {
                return err(StatusCode::CONFLICT, "already a member").into_response();
            }
            let member = json!({"id": Uuid::new_v4(), format!("{scope}_id"): p[scope], "user_id": uid,
                "role": body["role"], "user": user(&uid, name_of(&uid))});
            list.push(member.clone());
            (StatusCode::CREATED, Json(member)).into_response()
        },
    );
    let one = put(
        move |State(m): State<Shared>, Path(p): Params, Json(body): Json<Value>| async move {
            let mut m = m.lock().unwrap();
            let Some(x) = m
                .extra
                .get_mut(&format!("{prefix}:{}", p[scope]))
                .and_then(|l| l.iter_mut().find(|x| x["user_id"] == p["user"]))
            else {
                return not_found();
            };
            x["role"] = body["role"].clone();
            Json(x.clone()).into_response()
        },
    )
    .delete(move |State(m): State<Shared>, Path(p): Params| async move {
        let mut m = m.lock().unwrap();
        let Some(list) = m.extra.get_mut(&format!("{prefix}:{}", p[scope])) else {
            return not_found();
        };
        list.retain(|x| x["user_id"] != p["user"]);
        no_content()
    });
    (list, one)
}

async fn dashboard(
    State(m): State<Shared>,
    Path(p): Params,
    Query(q): Query<HashMap<String, String>>,
) -> Json<Value> {
    let m = m.lock().unwrap();
    let tenant = &p["tenant"];
    let project_of = |pid: &str| m.projects.iter().find(|x| x["id"] == pid).cloned();
    let mine: Vec<&Value> = m
        .tasks
        .iter()
        .filter(|t| {
            project_of(t["project_id"].as_str().unwrap_or_default())
                .is_some_and(|p| p["tenant_id"] == *tenant)
                && t["is_archived"] != true
                && t["assignees"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| x["user"]["id"] == USER_YUPIX))
        })
        .collect();
    let due = |t: &Value| {
        t["soft_deadline"]
            .as_str()
            .or(t["hard_deadline"].as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&chrono::Local).date_naive())
    };
    let open: Vec<&&Value> = mine.iter().filter(|t| t["status_id"] != ST_DONE).collect();
    let week_end = today() + chrono::Duration::days(6);
    let filter = q.get("filter").map(String::as_str).unwrap_or("all");
    let listed: Vec<Value> = open
        .iter()
        .filter(|t| match filter {
            "today" => due(t) == Some(today()),
            "overdue" => due(t).is_some_and(|d| d < today()),
            "week" => due(t).is_some_and(|d| d >= today() && d <= week_end),
            _ => true,
        })
        .map(|t| {
            let pr = project_of(t["project_id"].as_str().unwrap_or_default()).unwrap_or_default();
            json!({"done_status_id": ST_DONE, "task": {
                "id": t["id"], "seq_id": t["seq_id"],
                "seq_key": format!("{}-{}", pr["key"].as_str().unwrap_or("?"), t["seq_id"]),
                "title": t["title"], "priority": t["priority"],
                "status": status_summary(t["status_id"].as_str().unwrap_or_default()),
                "project": {"id": pr["id"], "key": pr["key"], "name": pr["name"], "is_personal": pr["is_personal"]},
                "is_personal": pr["is_personal"],
                "soft_deadline": t["soft_deadline"], "hard_deadline": t["hard_deadline"],
            }})
        })
        .collect();
    let projects: Vec<Value> = m
        .projects
        .iter()
        .filter(|p| p["tenant_id"] == *tenant)
        .map(|p| {
            let all: Vec<&Value> = m
                .tasks
                .iter()
                .filter(|t| t["project_id"] == p["id"])
                .collect();
            json!({"project": p, "total": all.len(),
                "completed": all.iter().filter(|t| t["status_id"] == ST_DONE).count()})
        })
        .collect();
    let activities: Vec<Value> = m
        .tasks
        .iter()
        .filter(|t| project_of(t["project_id"].as_str().unwrap_or_default()).is_some_and(|p| p["tenant_id"] == *tenant))
        .take(8)
        .map(|t| {
            let pr = project_of(t["project_id"].as_str().unwrap_or_default()).unwrap_or_default();
            json!({"project_key": pr["key"], "project_name": pr["name"], "task_seq_id": t["seq_id"],
                "task_title": t["title"], "activity": {"id": Uuid::new_v4(), "event_type": "task_created",
                "payload": {}, "user": {"id": USER_YUPIX, "name": "yupix"}, "created_at": t["created_at"]}})
        })
        .collect();
    let days: Vec<Value> = (0..7)
        .rev()
        .map(|n| json!({"date": (today() - chrono::Duration::days(n)).to_string(), "count": if n == 1 { 1 } else { 0 }}))
        .collect();
    Json(json!({
        "counts": {
            "open": open.len(),
            "today": open.iter().filter(|t| due(t) == Some(today())).count(),
            "overdue": open.iter().filter(|t| due(t).is_some_and(|d| d < today())).count(),
            "week": open.iter().filter(|t| due(t).is_some_and(|d| d >= today() && d <= week_end)).count(),
            "completed_week": mine.iter().filter(|t| t["status_id"] == ST_DONE).count(),
        },
        "total": listed.len(), "tasks": listed, "projects": projects,
        "activities": activities, "days": days,
    }))
}

fn status_summary(id: &str) -> Value {
    let (name, color) = match id {
        ST_BACKLOG => ("Backlog", "#8b949e"),
        ST_TODO => ("Todo", "#1f6feb"),
        ST_DOING => ("In Progress", "#d29922"),
        _ => ("Done", "#238636"),
    };
    json!({"id": id, "name": name, "color": color})
}

const DEFAULT_IN_APP: [&str; 8] = [
    "assigned",
    "mentioned",
    "status_changed",
    "deadline_soon",
    "comment_added",
    "pr_merged",
    "review_round_created",
    "review_finding_changed",
];

async fn get_notification_settings(State(m): State<Shared>, Path(p): Params) -> Json<Value> {
    let m = m.lock().unwrap();
    Json(
        m.notification_settings
            .get(&p["project"])
            .cloned()
            .unwrap_or_else(|| json!({"in_app_events": DEFAULT_IN_APP, "email_events": ["assigned", "mentioned"]})),
    )
}

async fn update_notification_settings(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    if !body["in_app_events"].is_array() || !body["email_events"].is_array() {
        return err(
            StatusCode::BAD_REQUEST,
            "in_app_events and email_events are required",
        )
        .into_response();
    }
    let v = json!({"in_app_events": body["in_app_events"], "email_events": body["email_events"]});
    m.lock()
        .unwrap()
        .notification_settings
        .insert(p["project"].clone(), v.clone());
    Json(v).into_response()
}

// ---- projects ----

pub(crate) fn tenant_projects(m: &Mock, tenant: &str) -> Vec<Value> {
    m.projects
        .iter()
        .filter(|p| p["tenant_id"] == tenant)
        .cloned()
        .collect()
}

async fn create_project(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    let Some(name) = body["name"].as_str().filter(|n| !n.trim().is_empty()) else {
        return err(StatusCode::BAD_REQUEST, "name is required").into_response();
    };
    let mut m = m.lock().unwrap();
    let key = body["key"]
        .as_str()
        .filter(|k| !k.is_empty())
        .map(str::to_uppercase)
        .unwrap_or_else(|| {
            name.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(4)
                .collect::<String>()
                .to_uppercase()
        });
    let key = if key.is_empty() {
        "PRJ".to_string()
    } else {
        key
    };
    if m.projects
        .iter()
        .any(|x| x["tenant_id"] == p["tenant"] && x["key"] == key)
    {
        return err(StatusCode::CONFLICT, "project key already exists").into_response();
    }
    let id = Uuid::new_v4().to_string();
    let project = json!({"id": id, "tenant_id": p["tenant"], "key": key, "name": name,
        "description": body["description"].as_str().unwrap_or_default(), "icon_url": null,
        "icon_emoji": body["icon_emoji"], "is_personal": false, "personal_owner_id": null});
    m.projects.push(project.clone());
    m.extra.insert(
        format!("project-members:{id}"),
        vec![
            json!({"id": Uuid::new_v4(), "project_id": id, "user_id": USER_YUPIX,
            "role": "Admin", "user": user(USER_YUPIX, "yupix")}),
        ],
    );
    (StatusCode::CREATED, Json(project)).into_response()
}

async fn get_project(State(m): State<Shared>, Path(p): Params) -> Response {
    let m = m.lock().unwrap();
    m.projects
        .iter()
        .find(|x| x["id"] == p["project"])
        .map(|x| Json(x.clone()).into_response())
        .unwrap_or_else(not_found)
}

async fn update_project(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    let mut m = m.lock().unwrap();
    let Some(x) = m.projects.iter_mut().find(|x| x["id"] == p["project"]) else {
        return not_found();
    };
    merge(x, &body);
    Json(x.clone()).into_response()
}

async fn delete_project(State(m): State<Shared>, Path(p): Params) -> Response {
    let mut m = m.lock().unwrap();
    let id = p["project"].clone();
    if m.projects
        .iter()
        .any(|x| x["id"] == id && x["is_personal"] == true)
    {
        return err(StatusCode::FORBIDDEN, "personal projects cannot be deleted").into_response();
    }
    let before = m.projects.len();
    m.projects.retain(|x| x["id"] != id);
    if m.projects.len() == before {
        return not_found();
    }
    m.tasks.retain(|t| t["project_id"] != id);
    no_content()
}

// ---- statuses / labels ----

/// 変更されるまでは既定の 4 列。
pub(crate) fn project_statuses<'a>(m: &'a mut Mock, project: &str) -> &'a mut Vec<Value> {
    m.extra
        .entry(format!("statuses:{project}"))
        .or_insert_with(|| {
            [
                (ST_BACKLOG, "Backlog", "#8b949e", 0, false, false),
                (ST_TODO, "Todo", "#1f6feb", 1, false, false),
                (ST_DOING, "In Progress", "#d29922", 2, false, false),
                (ST_DONE, "Done", "#238636", 3, true, true),
            ]
            .iter()
            .map(|(id, name, color, pos, done, dd)| {
                let mut s = status(id, name, color, *pos, *done, *dd);
                s["project_id"] = json!(project);
                s
            })
            .collect()
        })
}

async fn create_status(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    let mut m = m.lock().unwrap();
    let list = project_statuses(&mut m, &p["project"]);
    let mut s = status(
        &Uuid::new_v4().to_string(),
        body["name"].as_str().unwrap_or("New"),
        body["color"].as_str().unwrap_or("#8b949e"),
        list.len() as i32,
        body["is_done_state"] == true,
        body["is_default_done"] == true,
    );
    s["project_id"] = json!(p["project"]);
    s["is_default"] = json!(body["is_default"] == true);
    list.push(s.clone());
    (StatusCode::CREATED, Json(s)).into_response()
}

async fn update_status(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    let mut m = m.lock().unwrap();
    let list = project_statuses(&mut m, &p["project"]);
    // 既定 / 既定の完了は 1 つだけ。
    for flag in ["is_default", "is_default_done"] {
        if body[flag] == true {
            list.iter_mut().for_each(|s| s[flag] = json!(false));
        }
    }
    let Some(s) = list.iter_mut().find(|s| s["id"] == p["id"]) else {
        return not_found();
    };
    merge(s, &body);
    Json(s.clone()).into_response()
}

async fn delete_status(
    State(m): State<Shared>,
    Path(p): Params,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let mut m = m.lock().unwrap();
    let id = p["id"].clone();
    let in_use = m.tasks.iter().any(|t| t["status_id"] == id);
    match q.get("migrate_to_status_id") {
        Some(to) => {
            for t in m.tasks.iter_mut().filter(|t| t["status_id"] == id) {
                t["status_id"] = json!(to);
            }
        }
        None if in_use => {
            return err(
                StatusCode::CONFLICT,
                "status is in use; pass migrate_to_status_id",
            )
            .into_response();
        }
        None => {}
    }
    project_statuses(&mut m, &p["project"]).retain(|s| s["id"] != id);
    no_content()
}

async fn reorder_statuses(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    let mut m = m.lock().unwrap();
    let list = project_statuses(&mut m, &p["project"]);
    let ids: Vec<Value> = body["ids"].as_array().cloned().unwrap_or_default();
    for s in list.iter_mut() {
        if let Some(pos) = ids.iter().position(|id| *id == s["id"]) {
            s["position"] = json!(pos);
        }
    }
    list.sort_by_key(|s| s["position"].as_i64());
    Json(json!(list)).into_response()
}

async fn create_label(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    let mut m = m.lock().unwrap();
    let label = json!({"id": Uuid::new_v4(), "project_id": p["project"], "name": body["name"],
        "color": body["color"], "description": body["description"].as_str().unwrap_or_default(), "icon_url": null});
    m.labels.push(label.clone());
    (StatusCode::CREATED, Json(label)).into_response()
}

async fn update_label(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    let mut m = m.lock().unwrap();
    let Some(l) = m.labels.iter_mut().find(|l| l["id"] == p["id"]) else {
        return not_found();
    };
    merge(l, &body);
    let updated = l.clone();
    // タスクに付いた写しも合わせる。
    for t in m.tasks.iter_mut() {
        if let Some(labels) = t["labels"].as_array_mut() {
            for x in labels.iter_mut().filter(|x| x["id"] == updated["id"]) {
                *x = updated.clone();
            }
        }
    }
    Json(updated).into_response()
}

async fn delete_label(State(m): State<Shared>, Path(p): Params) -> Response {
    let mut m = m.lock().unwrap();
    let id = p["id"].clone();
    m.labels.retain(|l| l["id"] != id);
    for t in m.tasks.iter_mut() {
        if let Some(labels) = t["labels"].as_array_mut() {
            labels.retain(|x| x["id"] != id);
        }
    }
    no_content()
}

async fn export_labels(State(m): State<Shared>, Path(p): Params) -> Json<Value> {
    let m = m.lock().unwrap();
    let labels: Vec<Value> = m
        .labels
        .iter()
        .filter(|l| l["project_id"] == p["project"])
        .map(|l| json!({"name": l["name"], "color": l["color"], "description": l["description"]}))
        .collect();
    Json(json!({"version": 1, "labels": labels}))
}

async fn import_labels(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    let mut m = m.lock().unwrap();
    let mut out = vec![];
    for l in body["labels"].as_array().cloned().unwrap_or_default() {
        if m.labels
            .iter()
            .any(|x| x["project_id"] == p["project"] && x["name"] == l["name"])
        {
            continue;
        }
        let label = json!({"id": Uuid::new_v4(), "project_id": p["project"], "name": l["name"],
            "color": l["color"], "description": l["description"].as_str().unwrap_or_default(), "icon_url": null});
        m.labels.push(label.clone());
        out.push(label);
    }
    Json(json!(out)).into_response()
}

// ---- milestones / sprints (details) ----

async fn milestone_detail(State(m): State<Shared>, Path(p): Params) -> Response {
    let m = m.lock().unwrap();
    let Some(mut ms) = m
        .extra
        .get(&format!("milestones:{}", p["project"]))
        .and_then(|l| l.iter().find(|x| x["id"] == p["id"]).cloned())
    else {
        return not_found();
    };
    let tasks: Vec<&Value> = m
        .tasks
        .iter()
        .filter(|t| t["milestone_id"] == ms["id"])
        .collect();
    let done = tasks.iter().filter(|t| t["status_id"] == ST_DONE).count();
    ms["task_counts"] = json!({"total": tasks.len(), "done": done});
    ms["progress_pct"] = json!((done * 100).checked_div(tasks.len()).unwrap_or(0));
    Json(ms).into_response()
}

async fn sprint_detail(State(m): State<Shared>, Path(p): Params) -> Response {
    let m = m.lock().unwrap();
    let Some(mut sp) = m
        .extra
        .get(&format!("sprints:{}", p["project"]))
        .and_then(|l| l.iter().find(|x| x["id"] == p["id"]).cloned())
    else {
        return not_found();
    };
    let tasks: Vec<&Value> = m
        .tasks
        .iter()
        .filter(|t| t["sprint_id"] == sp["id"])
        .collect();
    let done = tasks.iter().filter(|t| t["status_id"] == ST_DONE).count();
    let doing = tasks.iter().filter(|t| t["status_id"] == ST_DOING).count();
    sp["task_counts"] = json!({"total": tasks.len(), "done": done, "in_progress": doing});
    let start = sp["start_date"]
        .as_str()
        .and_then(|s| s.parse::<chrono::NaiveDate>().ok())
        .unwrap_or(today());
    let end = sp["end_date"]
        .as_str()
        .and_then(|s| s.parse::<chrono::NaiveDate>().ok())
        .unwrap_or(today());
    let days = (end - start).num_days().max(1);
    let total = tasks.len() as i64;
    sp["burndown"] = json!(
        (0..=days)
            .map(|d| {
                let date = start + chrono::Duration::days(d);
                json!({"date": date.to_string(), "ideal_remaining": total - total * d / days,
                    "actual_remaining": if date <= today() { total - done as i64 } else { total }})
            })
            .collect::<Vec<_>>()
    );
    Json(sp).into_response()
}

fn sprint_mut<'a>(m: &'a mut Mock, p: &HashMap<String, String>) -> Option<&'a mut Value> {
    m.extra
        .get_mut(&format!("sprints:{}", p["project"]))
        .and_then(|l| l.iter_mut().find(|x| x["id"] == p["id"]))
}

async fn start_sprint(State(m): State<Shared>, Path(p): Params) -> Response {
    let mut m = m.lock().unwrap();
    let project = p["project"].clone();
    if m.extra.get(&format!("sprints:{project}")).is_some_and(|l| {
        l.iter()
            .any(|x| x["status"] == "active" && x["id"] != p["id"])
    }) {
        return err(StatusCode::CONFLICT, "another sprint is already active").into_response();
    }
    let Some(sp) = sprint_mut(&mut m, &p) else {
        return not_found();
    };
    if sp["status"] != "planning" {
        return err(StatusCode::CONFLICT, "only planning sprints can be started").into_response();
    }
    sp["status"] = json!("active");
    Json(sp.clone()).into_response()
}

async fn complete_sprint(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    let mut m = m.lock().unwrap();
    let Some(sp) = sprint_mut(&mut m, &p) else {
        return not_found();
    };
    if sp["status"] != "active" {
        return err(StatusCode::CONFLICT, "only active sprints can be completed").into_response();
    }
    sp["status"] = json!("completed");
    let done = sp.clone();
    let target = body["move_incomplete_to_sprint_id"].clone();
    for t in m
        .tasks
        .iter_mut()
        .filter(|t| t["sprint_id"] == done["id"] && t["status_id"] != ST_DONE)
    {
        t["sprint_id"] = target.clone();
    }
    Json(done).into_response()
}

async fn assign_tasks_to_sprint(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut m = m.lock().unwrap();
    let ids = body["task_ids"].as_array().cloned().unwrap_or_default();
    let sprint = json!(p["id"]);
    let mut out = vec![];
    for t in m.tasks.iter_mut().filter(|t| ids.contains(&t["id"])) {
        t["sprint_id"] = sprint.clone();
        out.push(t.clone());
    }
    Json(json!(out))
}

async fn github_integration() -> Json<Value> {
    Json(
        json!({"connected": true, "repo_owner": "koyori-app", "repo_name": "task",
        "connected_at": "2026-09-01T00:00:00Z"}),
    )
}

async fn github_install() -> Json<Value> {
    Json(json!({"url": "https://github.com/apps/koyori/installations/new"}))
}

// ---- tasks ----

fn task_id(p: &HashMap<String, String>) -> Option<Uuid> {
    p.get("task").and_then(|s| s.parse().ok())
}

async fn delete_task(State(m): State<Shared>, Path(p): Params) -> Response {
    let mut m = m.lock().unwrap();
    let Some(i) = task_id(&p).and_then(|id| find_task(&m, id)) else {
        return not_found();
    };
    m.tasks.remove(i);
    no_content()
}

async fn set_archived(m: Shared, p: HashMap<String, String>, archived: bool) -> Response {
    let mut m = m.lock().unwrap();
    let Some(i) = task_id(&p).and_then(|id| find_task(&m, id)) else {
        return not_found();
    };
    m.tasks[i]["is_archived"] = json!(archived);
    m.tasks[i]["updated_at"] = json!(now());
    let id = m.tasks[i]["id"].clone();
    let event = if archived { "archived" } else { "unarchived" };
    push_activity(&mut m, &id, event, json!({}));
    Json(m.tasks[i].clone()).into_response()
}

async fn bulk_update(State(m): State<Shared>, Json(body): Json<Value>) -> Json<Value> {
    let mut m = m.lock().unwrap();
    let ids = body["task_ids"].as_array().cloned().unwrap_or_default();
    let u = &body["update"];
    let labels = m.labels.clone();
    let mut updated = 0;
    let mut failed = vec![];
    for id in &ids {
        let Some(t) = m.tasks.iter_mut().find(|t| t["id"] == *id) else {
            failed.push(json!({"task_id": id, "reason": "not found"}));
            continue;
        };
        if !u["status_id"].is_null() {
            t["status_id"] = u["status_id"].clone();
        }
        if !u["sprint_id"].is_null() {
            t["sprint_id"] = u["sprint_id"].clone();
        }
        if u["clear_sprint_id"] == true {
            t["sprint_id"] = Value::Null;
        }
        if let Some(uid) = u["assignee_id"].as_str() {
            t["assignees"] = json!([{"role": "assignee", "user": user(uid, name_of(uid))}]);
        }
        let mut current = t["labels"].as_array().cloned().unwrap_or_default();
        for add in u["add_label_ids"].as_array().into_iter().flatten() {
            if !current.iter().any(|l| l["id"] == *add)
                && let Some(l) = labels.iter().find(|l| l["id"] == *add)
            {
                current.push(l.clone());
            }
        }
        if let Some(remove) = u["remove_label_ids"].as_array() {
            current.retain(|l| !remove.contains(&l["id"]));
        }
        t["labels"] = json!(current);
        t["updated_at"] = json!(now());
        updated += 1;
    }
    Json(json!({"updated": updated, "failed": failed}))
}

pub(crate) fn push_activity(m: &mut Mock, task: &Value, event: &str, payload: Value) {
    let key = format!("activities:{}", task.as_str().unwrap_or_default());
    m.extra.entry(key).or_default().insert(
        0,
        json!({"id": Uuid::new_v4(), "event_type": event, "payload": payload,
            "user": {"id": USER_YUPIX, "name": "yupix"}, "created_at": now()}),
    );
}

async fn activities(State(m): State<Shared>, Path(p): Params) -> Response {
    let m = m.lock().unwrap();
    let Some(i) = task_id(&p).and_then(|id| find_task(&m, id)) else {
        return not_found();
    };
    let t = &m.tasks[i];
    let mut items = m
        .extra
        .get(&format!("activities:{}", p["task"]))
        .cloned()
        .unwrap_or_default();
    items.push(
        json!({"id": Uuid::new_v4(), "event_type": "status_changed", "payload": {"to": "Todo"},
        "user": {"id": USER_ALICE, "name": "alice"}, "created_at": t["updated_at"]}),
    );
    items.push(
        json!({"id": Uuid::new_v4(), "event_type": "task_created", "payload": {},
        "user": {"id": USER_YUPIX, "name": "yupix"}, "created_at": t["created_at"]}),
    );
    Json(json!({"total": items.len(), "next_cursor": null, "activities": items})).into_response()
}

fn relation_entry(t: &Value, relation: &Value) -> Value {
    let mut e = t.clone();
    e["relation_id"] = relation["id"].clone();
    e
}

async fn relations(State(m): State<Shared>, Path(p): Params) -> Response {
    let m = m.lock().unwrap();
    let Some(i) = task_id(&p).and_then(|id| find_task(&m, id)) else {
        return not_found();
    };
    let me = &m.tasks[i];
    let rels = m
        .extra
        .get(&format!("relations:{}", p["project"]))
        .cloned()
        .unwrap_or_default();
    let find = |id: &Value| m.tasks.iter().find(|t| t["id"] == *id);
    let blocks: Vec<Value> = rels
        .iter()
        .filter(|r| r["blocker_task_id"] == me["id"])
        .filter_map(|r| find(&r["blocked_task_id"]).map(|t| relation_entry(t, r)))
        .collect();
    let blocked_by: Vec<Value> = rels
        .iter()
        .filter(|r| r["blocked_task_id"] == me["id"])
        .filter_map(|r| find(&r["blocker_task_id"]).map(|t| relation_entry(t, r)))
        .collect();
    let parent = find(&me["parent_task_id"]).cloned().unwrap_or(Value::Null);
    let subtasks: Vec<Value> = m
        .tasks
        .iter()
        .filter(|t| t["parent_task_id"] == me["id"])
        .cloned()
        .collect();
    Json(
        json!({"blocks": blocks, "blocked_by": blocked_by, "parent": parent, "subtasks": subtasks}),
    )
    .into_response()
}

async fn add_relation(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    let mut m = m.lock().unwrap();
    let me = json!(p["task"]);
    let other = body["target_task_id"].clone();
    let (blocker, blocked) = match body["type"].as_str() {
        Some("blocks") => (me.clone(), other),
        Some("blocked_by") => (other, me.clone()),
        _ => {
            return err(StatusCode::BAD_REQUEST, "type must be blocks or blocked_by")
                .into_response();
        }
    };
    if blocker == blocked {
        return err(StatusCode::BAD_REQUEST, "a task cannot block itself").into_response();
    }
    let rels = m
        .extra
        .entry(format!("relations:{}", p["project"]))
        .or_default();
    // 直接の逆向きだけを循環として拒否する（モックの簡易判定）。
    if rels
        .iter()
        .any(|r| r["blocker_task_id"] == blocked && r["blocked_task_id"] == blocker)
    {
        return err(StatusCode::CONFLICT, "this relation would create a cycle").into_response();
    }
    let rel = json!({"id": Uuid::new_v4(), "blocker_task_id": blocker, "blocked_task_id": blocked, "created_at": now()});
    rels.push(rel.clone());
    push_activity(&mut m, &me, "relation_added", json!({}));
    (StatusCode::CREATED, Json(rel)).into_response()
}

async fn remove_relation(State(m): State<Shared>, Path(p): Params) -> Response {
    let mut m = m.lock().unwrap();
    if let Some(rels) = m.extra.get_mut(&format!("relations:{}", p["project"])) {
        rels.retain(|r| r["id"] != p["relation"]);
    }
    push_activity(&mut m, &json!(p["task"]), "relation_removed", json!({}));
    no_content()
}

async fn time_summary(State(m): State<Shared>, Path(p): Params) -> Response {
    let m = m.lock().unwrap();
    let Some(i) = task_id(&p).and_then(|id| find_task(&m, id)) else {
        return not_found();
    };
    let logs = m
        .extra
        .get(&format!("time-logs:{}", p["task"]))
        .cloned()
        .unwrap_or_default();
    let actual: i64 = logs
        .iter()
        .filter_map(|l| l["logged_minutes"].as_i64())
        .sum();
    let estimate = m.tasks[i]["estimated_minutes"].as_i64();
    Json(json!({
        "actual_minutes": actual,
        "estimated_minutes": estimate,
        "remaining_minutes": estimate.map(|e| (e - actual).max(0)),
        "is_over": estimate.is_some_and(|e| actual > e),
        "by_user": if actual > 0 { json!([{"user_id": USER_YUPIX, "name": "yupix", "minutes": actual}]) } else { json!([]) },
    }))
    .into_response()
}

async fn timer_status(State(m): State<Shared>, Path(p): Params) -> Json<Value> {
    let m = m.lock().unwrap();
    match m.timers.get(&p["task"]) {
        Some(started) => {
            let elapsed = (chrono::Utc::now() - *started).num_minutes();
            Json(
                json!({"is_running": true, "started_at": rfc3339(*started), "elapsed_minutes": elapsed}),
            )
        }
        None => Json(json!({"is_running": false, "started_at": null, "elapsed_minutes": null})),
    }
}

async fn timer_start(State(m): State<Shared>, Path(p): Params) -> Response {
    let mut m = m.lock().unwrap();
    if m.timers.contains_key(&p["task"]) {
        return err(StatusCode::CONFLICT, "timer is already running").into_response();
    }
    let started = chrono::Utc::now();
    m.timers.insert(p["task"].clone(), started);
    (
        StatusCode::CREATED,
        Json(json!({"task_id": p["task"], "user_id": USER_YUPIX, "started_at": rfc3339(started)})),
    )
        .into_response()
}

async fn timer_stop(State(m): State<Shared>, Path(p): Params) -> Response {
    let mut m = m.lock().unwrap();
    let Some(started) = m.timers.remove(&p["task"]) else {
        return err(StatusCode::CONFLICT, "timer is not running").into_response();
    };
    let minutes = (chrono::Utc::now() - started).num_minutes().max(1);
    let log = json!({"id": Uuid::new_v4(), "task_id": p["task"], "user_id": USER_YUPIX,
        "logged_minutes": minutes, "logged_at": today().to_string(), "note": null, "created_at": now()});
    m.extra
        .entry(format!("time-logs:{}", p["task"]))
        .or_default()
        .push(log.clone());
    Json(log).into_response()
}

async fn watchers(State(m): State<Shared>, Path(p): Params) -> Json<Value> {
    let m = m.lock().unwrap();
    Json(
        json!({"watchers": m.extra.get(&format!("watchers:{}", p["task"])).cloned().unwrap_or_default()}),
    )
}

async fn watch(State(m): State<Shared>, Path(p): Params) -> Response {
    let mut m = m.lock().unwrap();
    let list = m
        .extra
        .entry(format!("watchers:{}", p["task"]))
        .or_default();
    if !list.iter().any(|w| w["id"] == USER_YUPIX) {
        list.push(json!({"id": USER_YUPIX, "name": "yupix", "created_at": now()}));
    }
    StatusCode::CREATED.into_response()
}

async fn unwatch(State(m): State<Shared>, Path(p): Params) -> Response {
    let mut m = m.lock().unwrap();
    if let Some(list) = m.extra.get_mut(&format!("watchers:{}", p["task"])) {
        list.retain(|w| w["id"] != USER_YUPIX);
    }
    no_content()
}

async fn attachments(State(m): State<Shared>, Path(p): Params) -> Json<Value> {
    let m = m.lock().unwrap();
    Json(
        json!({"attachments": m.extra.get(&format!("attachments:{}", p["task"])).cloned().unwrap_or_default()}),
    )
}

async fn detach(State(m): State<Shared>, Path(p): Params) -> Response {
    let mut m = m.lock().unwrap();
    if let Some(list) = m.extra.get_mut(&format!("attachments:{}", p["task"])) {
        list.retain(|a| a["id"] != p["attachment"]);
    }
    no_content()
}

/// コメント（返信を含む）を id で探す。
fn comment_mut<'a>(m: &'a mut Mock, id: &str) -> Option<&'a mut Value> {
    for c in m.comments.iter_mut() {
        if c["id"] == id {
            return Some(c);
        }
        if let Some(r) = c["replies"]
            .as_array_mut()
            .and_then(|rs| rs.iter_mut().find(|r| r["id"] == id))
        {
            return Some(r);
        }
    }
    None
}

async fn update_comment(
    State(m): State<Shared>,
    Path(p): Params,
    Json(body): Json<Value>,
) -> Response {
    let Some(text) = body["body"].as_str().filter(|b| !b.trim().is_empty()) else {
        return err(StatusCode::BAD_REQUEST, "Comment body is required").into_response();
    };
    let mut m = m.lock().unwrap();
    let task = p["task"].clone();
    let Some(c) = comment_mut(&mut m, &p["comment"]) else {
        return not_found();
    };
    if c["user"]["id"] != USER_YUPIX {
        return err(
            StatusCode::FORBIDDEN,
            "only the author can edit this comment",
        )
        .into_response();
    }
    c["body"] = json!(text);
    c["updated_at"] = json!(now());
    let out = json!({"id": c["id"], "task_id": task, "body": text, "user_id": USER_YUPIX,
        "parent_comment_id": null, "created_at": c["created_at"], "updated_at": c["updated_at"], "deleted_at": null});
    push_activity(&mut m, &json!(task), "comment_edited", json!({}));
    Json(out).into_response()
}

async fn delete_comment(State(m): State<Shared>, Path(p): Params) -> Response {
    let mut m = m.lock().unwrap();
    let Some(c) = comment_mut(&mut m, &p["comment"]) else {
        return not_found();
    };
    c["is_deleted"] = json!(true);
    c["body"] = Value::Null;
    push_activity(&mut m, &json!(p["task"]), "comment_deleted", json!({}));
    no_content()
}

pub(crate) fn routes(t: &str, tp: &str) -> Router<Shared> {
    let task = format!("{tp}/tasks/{{task}}");
    let (tenant_members, tenant_member) = members_router("tenant", "tenant-members");
    let (project_members, project_member) = members_router("project", "project-members");
    Router::new()
        .route("/v1/auth/me", get(get_me).patch(update_me))
        .route(
            "/v1/users/me/notification-settings/{project}",
            get(get_notification_settings).put(update_notification_settings),
        )
        .route(t, get(get_tenant).put(update_tenant))
        .route(&format!("{t}/members"), tenant_members)
        .route(&format!("{t}/members/{{user}}"), tenant_member)
        .route(&format!("{t}/users/me/dashboard"), get(dashboard))
        .route(&format!("{t}/projects"), post(create_project))
        .route(
            tp,
            get(get_project).put(update_project).delete(delete_project),
        )
        .route(&format!("{tp}/members"), project_members)
        .route(&format!("{tp}/members/{{user}}"), project_member)
        .route(&format!("{tp}/statuses"), post(create_status))
        .route(&format!("{tp}/statuses/reorder"), put(reorder_statuses))
        .route(
            &format!("{tp}/statuses/{{id}}"),
            put(update_status).delete(delete_status),
        )
        .route(&format!("{tp}/labels"), post(create_label))
        .route(&format!("{tp}/labels/export"), get(export_labels))
        .route(&format!("{tp}/labels/import"), post(import_labels))
        .route(
            &format!("{tp}/labels/{{id}}"),
            put(update_label).delete(delete_label),
        )
        .route(&format!("{tp}/milestones"), collection(&MILESTONES))
        .route(
            &format!("{tp}/milestones/{{id}}"),
            item(&MILESTONES, "id").get(milestone_detail),
        )
        .route(&format!("{tp}/sprints"), collection(&SPRINTS))
        .route(
            &format!("{tp}/sprints/{{id}}"),
            item(&SPRINTS, "id").get(sprint_detail),
        )
        .route(&format!("{tp}/sprints/{{id}}/start"), post(start_sprint))
        .route(
            &format!("{tp}/sprints/{{id}}/complete"),
            post(complete_sprint),
        )
        .route(
            &format!("{tp}/sprints/{{id}}/tasks"),
            post(assign_tasks_to_sprint),
        )
        .route(&format!("{tp}/custom-fields"), collection(&CUSTOM_FIELDS))
        .route(
            &format!("{tp}/custom-fields/{{id}}"),
            item(&CUSTOM_FIELDS, "id"),
        )
        .route(&format!("{tp}/task-views"), collection(&TASK_VIEWS))
        .route(
            &format!("{tp}/task-views/{{id}}"),
            item(&TASK_VIEWS, "id").merge(item_get(&TASK_VIEWS, "id")),
        )
        .route(&format!("{tp}/github/integration"), get(github_integration))
        .route(&format!("{tp}/github/install"), get(github_install))
        .route(&format!("{tp}/tasks/bulk"), post(bulk_update))
        .route(
            &format!("{task}/archive"),
            post(|State(m): State<Shared>, Path(p): Params| set_archived(m, p, true)),
        )
        .route(
            &format!("{task}/unarchive"),
            post(|State(m): State<Shared>, Path(p): Params| set_archived(m, p, false)),
        )
        .route(&format!("{task}/activities"), get(activities))
        .route(
            &format!("{task}/relations"),
            get(relations).post(add_relation),
        )
        .route(
            &format!("{task}/relations/{{relation}}"),
            axum::routing::delete(remove_relation),
        )
        .route(&format!("{task}/time-logs"), collection(&TIME_LOGS))
        .route(&format!("{task}/time-logs/summary"), get(time_summary))
        .route(
            &format!("{task}/time-logs/{{id}}"),
            item(&TIME_LOGS, "id").merge(item_get(&TIME_LOGS, "id")),
        )
        .route(&format!("{task}/timer/status"), get(timer_status))
        .route(&format!("{task}/timer/start"), post(timer_start))
        .route(&format!("{task}/timer/stop"), post(timer_stop))
        .route(&format!("{task}/watchers"), get(watchers))
        .route(&format!("{task}/watch"), post(watch).delete(unwatch))
        .route(&format!("{task}/attachments"), get(attachments))
        .route(
            &format!("{task}/attachments/{{attachment}}"),
            axum::routing::delete(detach),
        )
        .route(
            &format!("{task}/comments/{{comment}}"),
            put(update_comment).delete(delete_comment),
        )
        .route(&task, axum::routing::delete(delete_task))
}
