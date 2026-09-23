use std::{future::Future, sync::OnceLock};

use reqwest::{Method, StatusCode, Url};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::{
    error::{ApiError, Result},
    spec, types,
};

/// reqwest futures need a tokio reactor; GPUI runs on its own executor, so
/// API work is always spawned here and the JoinHandle is awaited by callers.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

async fn on_runtime<F, T>(fut: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    runtime()
        .spawn(fut)
        .await
        .expect("api request task panicked")
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ErrorBody {
    message: String,
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: Url,
    token: std::sync::Arc<str>,
    unauthorized: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Client {
    /// `base_url` は `https://host/api` まで（`/v1` は各メソッドが付ける）。
    pub fn new(base_url: &str, token: impl AsRef<str>) -> Result<Self> {
        let base = Url::parse(base_url)
            .map_err(|e| ApiError::InvalidConfig(format!("{base_url}: {e}")))?;
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .default_headers({
                let mut h = reqwest::header::HeaderMap::new();
                h.insert(reqwest::header::ACCEPT, "application/json".parse().unwrap());
                h
            })
            .build()?;
        Ok(Self {
            http,
            base,
            token: token.as_ref().into(),
            unauthorized: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// Shared by all cloned clients so any feature's 401 expires the UI session.
    pub fn is_unauthorized(&self) -> bool {
        self.unauthorized.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Clones share session identity; a newly authenticated client does not.
    pub fn same_session(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.unauthorized, &other.unauthorized)
    }

    fn url(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| ApiError::InvalidConfig("api base must not be cannot-be-a-base".into()))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    async fn send<T>(
        &self,
        method: Method,
        segments: &[&str],
        query: &[(String, String)],
        body: Option<serde_json::Value>,
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let url = self.url(segments)?;
        let mut req = self.http.request(method, url).query(query);
        if !self.token.is_empty() {
            req = req.bearer_auth(&*self.token);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status == StatusCode::UNAUTHORIZED {
            self.unauthorized
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if status.is_success() {
            let text = resp.text().await?;
            let text = if text.trim().is_empty() {
                "null"
            } else {
                &text
            };
            return Ok(serde_json::from_str(text)?);
        }
        let message = resp
            .json::<ErrorBody>()
            .await
            .map(|e| e.message)
            .unwrap_or_else(|_| {
                status
                    .canonical_reason()
                    .unwrap_or("unknown error")
                    .to_string()
            });
        Err(match status {
            StatusCode::UNAUTHORIZED => ApiError::Unauthorized,
            StatusCode::FORBIDDEN => ApiError::Forbidden { message },
            StatusCode::NOT_FOUND => ApiError::NotFound,
            StatusCode::CONFLICT => ApiError::Conflict { message },
            s if s.is_client_error() => ApiError::BadRequest { message },
            _ => ApiError::Status {
                status: status.as_u16(),
                message,
            },
        })
    }

    async fn send_unit(
        &self,
        method: Method,
        segments: &[&str],
        body: Option<serde_json::Value>,
    ) -> Result<()> {
        // 応答の形を気にしない終わり系（204 / 確認応答）に使う
        self.send::<serde_json::Value>(method, segments, &[], body)
            .await?;
        Ok(())
    }

    // ---- desktop auth (task.md §17) ----

    /// POST /v1/desktop/auth/token — use a Client with an empty token to omit
    /// Authorization at this unauthenticated exchange endpoint.
    pub async fn exchange_desktop_code(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<spec::DesktopAuthTokenResponse> {
        let c = self.clone();
        let body = serde_json::to_value(spec::DesktopAuthTokenRequest {
            code: code.to_owned(),
            code_verifier: code_verifier.to_owned(),
        })?;
        on_runtime(async move {
            c.send(
                Method::POST,
                &["v1", "desktop", "auth", "token"],
                &[],
                Some(body),
            )
            .await
        })
        .await
    }

    /// GET /v1/users/me/devices
    pub async fn list_devices(&self) -> Result<spec::DeviceList> {
        let c = self.clone();
        on_runtime(async move {
            c.send(Method::GET, &["v1", "users", "me", "devices"], &[], None)
                .await
        })
        .await
    }

    /// DELETE /v1/users/me/devices/{id} — 自分自身の削除 = ログアウト。
    pub async fn delete_device(&self, id: Uuid) -> Result<()> {
        let c = self.clone();
        on_runtime(async move {
            c.send_unit(
                Method::DELETE,
                &["v1", "users", "me", "devices", &id.to_string()],
                None,
            )
            .await
        })
        .await
    }

    // ---- notifications (task.md §9) ----

    pub async fn list_notifications(
        &self,
        query: &NotificationsQuery,
    ) -> Result<spec::NotificationList> {
        let c = self.clone();
        let query = query.to_pairs();
        on_runtime(async move {
            c.send(
                Method::GET,
                &["v1", "users", "me", "notifications"],
                &query,
                None,
            )
            .await
        })
        .await
    }

    pub async fn mark_notification_read(&self, id: Uuid) -> Result<()> {
        let c = self.clone();
        on_runtime(async move {
            c.send_unit(
                Method::PATCH,
                &[
                    "v1",
                    "users",
                    "me",
                    "notifications",
                    &id.to_string(),
                    "read",
                ],
                None,
            )
            .await
        })
        .await
    }

    pub async fn mark_all_notifications_read(&self) -> Result<()> {
        let c = self.clone();
        on_runtime(async move {
            c.send_unit(
                Method::PATCH,
                &["v1", "users", "me", "notifications", "read-all"],
                None,
            )
            .await
        })
        .await
    }

    // ---- tenants / projects ----

    pub async fn get_me(&self) -> Result<types::UserResponse> {
        let c = self.clone();
        on_runtime(async move { c.send(Method::GET, &["v1", "auth", "me"], &[], None).await }).await
    }

    pub async fn list_tenants(&self) -> Result<Vec<types::TenantListItemResponse>> {
        let c = self.clone();
        on_runtime(async move { c.send(Method::GET, &["v1", "tenants"], &[], None).await }).await
    }

    pub async fn list_projects(&self, tenant: Uuid) -> Result<Vec<types::ProjectResponse>> {
        let c = self.clone();
        on_runtime(async move {
            c.send(
                Method::GET,
                &["v1", "tenants", &tenant.to_string(), "projects"],
                &[],
                None,
            )
            .await
        })
        .await
    }

    pub async fn get_personal_project(&self, tenant: Uuid) -> Result<types::ProjectResponse> {
        let c = self.clone();
        on_runtime(async move {
            c.send(
                Method::GET,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "users",
                    "me",
                    "personal-project",
                ],
                &[],
                None,
            )
            .await
        })
        .await
    }

    pub async fn list_statuses(
        &self,
        tenant: Uuid,
        project: Uuid,
    ) -> Result<Vec<types::ProjectStatusResponse>> {
        let c = self.clone();
        on_runtime(async move {
            c.send(
                Method::GET,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "statuses",
                ],
                &[],
                None,
            )
            .await
        })
        .await
    }

    pub async fn list_assignable_users(
        &self,
        tenant: Uuid,
        project: Uuid,
        username: Option<&str>,
    ) -> Result<Vec<types::UserSummary>> {
        let c = self.clone();
        let mut query = vec![];
        if let Some(u) = username {
            query.push(("username".to_string(), u.to_string()));
        }
        on_runtime(async move {
            c.send(
                Method::GET,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "assignable-users",
                ],
                &query,
                None,
            )
            .await
        })
        .await
    }

    // ---- tasks ----

    pub async fn search_tasks(
        &self,
        tenant: Uuid,
        project: Uuid,
        query: &str,
    ) -> Result<types::SearchTasksResponse> {
        let c = self.clone();
        let query = vec![
            ("q".into(), query.to_owned()),
            ("limit".into(), "50".into()),
        ];
        on_runtime(async move {
            c.send(
                Method::GET,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "tasks",
                    "search",
                ],
                &query,
                None,
            )
            .await
        })
        .await
    }

    pub async fn list_my_tasks(
        &self,
        tenant: Uuid,
        query: &MyTasksQuery,
    ) -> Result<types::MyTasksListResponse> {
        let c = self.clone();
        let query = query.to_pairs();
        on_runtime(async move {
            c.send(
                Method::GET,
                &["v1", "tenants", &tenant.to_string(), "users", "me", "tasks"],
                &query,
                None,
            )
            .await
        })
        .await
    }

    pub async fn list_tasks(
        &self,
        tenant: Uuid,
        project: Uuid,
        query: &TasksQuery,
    ) -> Result<types::TaskListResponse> {
        let c = self.clone();
        let query = query.to_pairs();
        on_runtime(async move {
            c.send(
                Method::GET,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "tasks",
                ],
                &query,
                None,
            )
            .await
        })
        .await
    }

    pub async fn get_task(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<types::TaskDetailResponse> {
        let c = self.clone();
        on_runtime(async move {
            c.send(
                Method::GET,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "tasks",
                    &task.to_string(),
                ],
                &[],
                None,
            )
            .await
        })
        .await
    }

    pub async fn create_task(
        &self,
        tenant: Uuid,
        project: Uuid,
        body: &types::CreateTaskRequest,
    ) -> Result<types::TaskDetailResponse> {
        let c = self.clone();
        let body = serde_json::to_value(body)?;
        on_runtime(async move {
            c.send(
                Method::POST,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "tasks",
                ],
                &[],
                Some(body),
            )
            .await
        })
        .await
    }

    /// PUT（PATCH ではない）で部分更新。`Option` 項目は送らない限り既存値を維持。
    pub async fn update_task(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        body: &types::UpdateTaskRequest,
    ) -> Result<types::TaskDetailResponse> {
        let c = self.clone();
        let body = serde_json::to_value(body)?;
        on_runtime(async move {
            c.send(
                Method::PUT,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "tasks",
                    &task.to_string(),
                ],
                &[],
                Some(body),
            )
            .await
        })
        .await
    }

    pub async fn list_comments(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<types::CommentListResponse> {
        let c = self.clone();
        on_runtime(async move {
            c.send(
                Method::GET,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "tasks",
                    &task.to_string(),
                    "comments",
                ],
                &[],
                None,
            )
            .await
        })
        .await
    }

    pub async fn create_comment(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        body: &types::CreateCommentRequest,
    ) -> Result<types::TaskCommentResponse> {
        let c = self.clone();
        let body = serde_json::to_value(body)?;
        on_runtime(async move {
            c.send(
                Method::POST,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "tasks",
                    &task.to_string(),
                    "comments",
                ],
                &[],
                Some(body),
            )
            .await
        })
        .await
    }

    // ---- reviews ----

    pub async fn list_reviewed_pull_requests(
        &self,
        tenant: Uuid,
        project: Uuid,
    ) -> Result<Vec<types::ReviewedPullRequest>> {
        let c = self.clone();
        on_runtime(async move {
            c.send(
                Method::GET,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "reviews",
                    "pull-requests",
                ],
                &[],
                None,
            )
            .await
        })
        .await
    }

    pub async fn list_reviews(
        &self,
        tenant: Uuid,
        project: Uuid,
        pr: Option<i64>,
        repo: Option<&str>,
    ) -> Result<Vec<types::ReviewResponse>> {
        let c = self.clone();
        let mut query = vec![];
        if let Some(pr) = pr {
            query.push(("pr".to_string(), pr.to_string()));
        }
        if let Some(r) = repo {
            query.push(("repo".to_string(), r.to_string()));
        }
        on_runtime(async move {
            c.send(
                Method::GET,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "reviews",
                ],
                &query,
                None,
            )
            .await
        })
        .await
    }

    pub async fn get_review(
        &self,
        tenant: Uuid,
        project: Uuid,
        review: Uuid,
    ) -> Result<types::ReviewDetailResponse> {
        let c = self.clone();
        on_runtime(async move {
            c.send(
                Method::GET,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "reviews",
                    &review.to_string(),
                ],
                &[],
                None,
            )
            .await
        })
        .await
    }

    /// `GET …/reviews/summary`。`gate` は task.md §18.2 の追加フィールドで、
    /// 本番が返すまで `None`（`ReviewSummary::gate` は Option）。
    pub async fn get_review_summary(
        &self,
        tenant: Uuid,
        project: Uuid,
        pr: i64,
        repo: Option<&str>,
    ) -> Result<spec::ReviewSummary> {
        let c = self.clone();
        let mut query = vec![("pr".to_string(), pr.to_string())];
        if let Some(r) = repo {
            query.push(("repo".to_string(), r.to_string()));
        }
        on_runtime(async move {
            c.send(
                Method::GET,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "reviews",
                    "summary",
                ],
                &query,
                None,
            )
            .await
        })
        .await
    }

    pub async fn list_review_findings(
        &self,
        tenant: Uuid,
        project: Uuid,
        filter: &FindingsQuery,
    ) -> Result<Vec<spec::Finding>> {
        let c = self.clone();
        let query = filter.to_pairs();
        on_runtime(async move {
            c.send(
                Method::GET,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "review-findings",
                ],
                &query,
                None,
            )
            .await
        })
        .await
    }

    /// PATCH …/review-findings/{id} {state, note}。403/409 は backend の
    /// message をそのまま ApiError へ載せる（§16.5 / §23）。
    pub async fn update_finding_state(
        &self,
        tenant: Uuid,
        project: Uuid,
        finding: Uuid,
        state: types::FindingState,
        note: Option<String>,
    ) -> Result<spec::Finding> {
        let c = self.clone();
        let body = serde_json::to_value(&types::UpdateFindingStateRequest { note, state })?;
        on_runtime(async move {
            c.send(
                Method::PATCH,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "review-findings",
                    &finding.to_string(),
                ],
                &[],
                Some(body),
            )
            .await
        })
        .await
    }

    /// POST …/reviews — Draft の指摘を Round + Findings として一括確定。
    /// `head_sha` の 40 桁検証は呼び出し側の責務（§16.7）。
    pub async fn create_review(
        &self,
        tenant: Uuid,
        project: Uuid,
        body: &types::CreateReviewRequest,
    ) -> Result<types::ReviewDetailResponse> {
        self.create_review_with_repository(tenant, project, body, None, None)
            .await
    }

    pub async fn create_review_with_repository(
        &self,
        tenant: Uuid,
        project: Uuid,
        body: &types::CreateReviewRequest,
        repo: Option<&str>,
        host: Option<&str>,
    ) -> Result<types::ReviewDetailResponse> {
        let c = self.clone();
        let mut body = serde_json::to_value(body)?;
        if let Some(repo) = repo.filter(|s| !s.is_empty()) {
            body["repo"] = serde_json::json!(repo);
        }
        if let Some(host) = host.filter(|s| !s.is_empty()) {
            body["host"] = serde_json::json!(host);
        }
        on_runtime(async move {
            c.send(
                Method::POST,
                &[
                    "v1",
                    "tenants",
                    &tenant.to_string(),
                    "projects",
                    &project.to_string(),
                    "reviews",
                ],
                &[],
                Some(body),
            )
            .await
        })
        .await
    }
}

// ---- query builders ----

#[derive(Debug, Clone, Default)]
pub struct NotificationsQuery {
    pub unread: Option<bool>,
    pub kind: Option<spec::NotificationKind>,
    pub limit: Option<u32>,
    /// 通常のページ送り（このカーソルより古い行）。
    pub cursor: Option<String>,
    /// catch-up（このカーソルより新しい行）。cursor と同時指定は backend が 400。
    pub after: Option<String>,
}

impl NotificationsQuery {
    fn to_pairs(&self) -> Vec<(String, String)> {
        let mut v = vec![];
        if let Some(x) = self.unread {
            v.push(("unread".into(), x.to_string()));
        }
        if let Some(k) = &self.kind {
            v.push(("kind".into(), k.as_str().to_string()));
        }
        if let Some(x) = self.limit {
            v.push(("limit".into(), x.to_string()));
        }
        if let Some(x) = &self.cursor {
            v.push(("cursor".into(), x.clone()));
        }
        if let Some(x) = &self.after {
            v.push(("after".into(), x.clone()));
        }
        v
    }
}

#[derive(Debug, Clone, Default)]
pub struct MyTasksQuery {
    pub filter: Option<String>,
    pub include_personal: Option<bool>,
    pub project_id: Option<Uuid>,
    pub limit: Option<u32>,
    pub offset: Option<u64>,
}

impl MyTasksQuery {
    fn to_pairs(&self) -> Vec<(String, String)> {
        let mut v = vec![];
        if let Some(x) = &self.filter {
            v.push(("filter".into(), x.clone()));
        }
        if let Some(x) = self.include_personal {
            v.push(("include_personal".into(), x.to_string()));
        }
        if let Some(x) = self.project_id {
            v.push(("project_id".into(), x.to_string()));
        }
        if let Some(x) = self.limit {
            v.push(("limit".into(), x.to_string()));
        }
        if let Some(x) = self.offset {
            v.push(("offset".into(), x.to_string()));
        }
        v
    }
}

#[derive(Debug, Clone, Default)]
pub struct TasksQuery {
    pub status_id: Option<Uuid>,
    pub priority: Option<types::TaskPriority>,
    pub assignee_id: Option<Uuid>,
    pub label_id: Option<Uuid>,
    pub milestone_id: Option<Uuid>,
    pub sprint_id: Option<Uuid>,
    pub parent_task_id: Option<Uuid>,
    pub root_only: Option<bool>,
    pub is_archived: Option<bool>,
    pub sort: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u64>,
    pub cursor: Option<String>,
}

impl TasksQuery {
    fn to_pairs(&self) -> Vec<(String, String)> {
        let mut v = vec![];
        macro_rules! q {
            ($f:ident) => {
                if let Some(x) = &self.$f {
                    v.push((stringify!($f).into(), x.to_string()));
                }
            };
        }
        q!(status_id);
        q!(assignee_id);
        q!(label_id);
        q!(milestone_id);
        q!(sprint_id);
        q!(parent_task_id);
        q!(root_only);
        q!(is_archived);
        q!(sort);
        q!(limit);
        q!(offset);
        q!(cursor);
        if let Some(p) = &self.priority {
            v.push((
                "priority".into(),
                serde_json::to_string(p)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_string(),
            ));
        }
        v
    }
}

#[derive(Debug, Clone, Default)]
pub struct FindingsQuery {
    pub pr: Option<i64>,
    pub repo: Option<String>,
    pub state: Option<types::FindingState>,
    pub severity: Option<types::FindingSeverity>,
}

impl FindingsQuery {
    fn to_pairs(&self) -> Vec<(String, String)> {
        let mut v = vec![];
        if let Some(x) = self.pr {
            v.push(("pr".into(), x.to_string()));
        }
        if let Some(x) = &self.repo {
            v.push(("repo".into(), x.clone()));
        }
        if let Some(x) = &self.state {
            v.push((
                "state".into(),
                serde_json::to_string(x)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_string(),
            ));
        }
        if let Some(x) = &self.severity {
            v.push((
                "severity".into(),
                serde_json::to_string(x)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_string(),
            ));
        }
        v
    }
}
