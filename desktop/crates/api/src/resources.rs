//! MVP 以降に足したエンドポイント（プロジェクト管理・タスクの付帯情報・ダッシュボード等）。
//! 経路は openapi.json のとおり。`mock-api` のテストで全メソッドの経路を spec と突き合わせる。

use reqwest::Method;
use serde::Serialize;
use uuid::Uuid;

use crate::{Client, error::Result, types};

fn tenant_path(tenant: Uuid, rest: &[&str]) -> Vec<String> {
    let mut v = vec!["v1".into(), "tenants".into(), tenant.to_string()];
    v.extend(rest.iter().map(|s| s.to_string()));
    v
}

fn project_path(tenant: Uuid, project: Uuid, rest: &[&str]) -> Vec<String> {
    let mut v = tenant_path(tenant, &["projects", &project.to_string()]);
    v.extend(rest.iter().map(|s| s.to_string()));
    v
}

fn task_path(tenant: Uuid, project: Uuid, task: Uuid, rest: &[&str]) -> Vec<String> {
    let mut v = project_path(tenant, project, &["tasks", &task.to_string()]);
    v.extend(rest.iter().map(|s| s.to_string()));
    v
}

fn body<T: Serialize>(value: &T) -> Result<Option<serde_json::Value>> {
    Ok(Some(serde_json::to_value(value)?))
}

fn opt_pairs(pairs: &[(&str, Option<String>)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .filter_map(|(k, v)| v.clone().map(|v| (k.to_string(), v)))
        .collect()
}

/// `GET …/users/me/dashboard` の絞り込み。
#[derive(Debug, Clone, Default)]
pub struct DashboardQuery {
    /// today / week / overdue / all
    pub filter: Option<String>,
    /// IANA タイムゾーン名。
    pub timezone: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u64>,
}

impl Client {
    // ---- account / tenant ----

    /// PATCH /v1/auth/me
    pub async fn update_me(
        &self,
        req: &types::UpdateProfileRequest,
    ) -> Result<types::UserResponse> {
        self.request(
            Method::PATCH,
            vec!["v1".into(), "auth".into(), "me".into()],
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn get_tenant(&self, tenant: Uuid) -> Result<types::TenantResponse> {
        self.request(Method::GET, tenant_path(tenant, &[]), vec![], None)
            .await
    }

    pub async fn update_tenant(
        &self,
        tenant: Uuid,
        req: &types::UpdateTenantRequest,
    ) -> Result<types::TenantResponse> {
        self.request(Method::PUT, tenant_path(tenant, &[]), vec![], body(req)?)
            .await
    }

    pub async fn list_tenant_members(
        &self,
        tenant: Uuid,
    ) -> Result<Vec<types::TenantMemberResponse>> {
        self.request(Method::GET, tenant_path(tenant, &["members"]), vec![], None)
            .await
    }

    pub async fn add_tenant_member(
        &self,
        tenant: Uuid,
        req: &types::AddTenantMemberRequest,
    ) -> Result<types::TenantMemberResponse> {
        self.request(
            Method::POST,
            tenant_path(tenant, &["members"]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn update_tenant_member(
        &self,
        tenant: Uuid,
        user: Uuid,
        role: types::TenantRole,
    ) -> Result<types::TenantMemberResponse> {
        self.request(
            Method::PUT,
            tenant_path(tenant, &["members", &user.to_string()]),
            vec![],
            body(&types::UpdateTenantMemberRequest { role })?,
        )
        .await
    }

    pub async fn remove_tenant_member(&self, tenant: Uuid, user: Uuid) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            tenant_path(tenant, &["members", &user.to_string()]),
            vec![],
            None,
        )
        .await
    }

    /// GET …/users/me/dashboard — ホームの件数・進捗・最近の更新。
    pub async fn get_dashboard(
        &self,
        tenant: Uuid,
        query: &DashboardQuery,
    ) -> Result<types::DashboardResponse> {
        let query = opt_pairs(&[
            ("filter", query.filter.clone()),
            ("timezone", query.timezone.clone()),
            ("limit", query.limit.map(|v| v.to_string())),
            ("offset", query.offset.map(|v| v.to_string())),
        ]);
        self.request(
            Method::GET,
            tenant_path(tenant, &["users", "me", "dashboard"]),
            query,
            None,
        )
        .await
    }

    /// GET /v1/users/me/notification-settings/{project_id}
    pub async fn get_notification_settings(
        &self,
        project: Uuid,
    ) -> Result<types::NotificationSettingsResponse> {
        self.request(
            Method::GET,
            vec![
                "v1".into(),
                "users".into(),
                "me".into(),
                "notification-settings".into(),
                project.to_string(),
            ],
            vec![],
            None,
        )
        .await
    }

    /// PUT /v1/users/me/notification-settings/{project_id}（応答と同じ形で全体を置き換える）。
    pub async fn update_notification_settings(
        &self,
        project: Uuid,
        req: &types::NotificationSettingsResponse,
    ) -> Result<types::NotificationSettingsResponse> {
        self.request(
            Method::PUT,
            vec![
                "v1".into(),
                "users".into(),
                "me".into(),
                "notification-settings".into(),
                project.to_string(),
            ],
            vec![],
            body(req)?,
        )
        .await
    }

    // ---- projects ----

    pub async fn get_project(&self, tenant: Uuid, project: Uuid) -> Result<types::ProjectResponse> {
        self.request(
            Method::GET,
            project_path(tenant, project, &[]),
            vec![],
            None,
        )
        .await
    }

    pub async fn create_project(
        &self,
        tenant: Uuid,
        req: &types::CreateProjectRequest,
    ) -> Result<types::ProjectResponse> {
        self.request(
            Method::POST,
            tenant_path(tenant, &["projects"]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn update_project(
        &self,
        tenant: Uuid,
        project: Uuid,
        req: &types::UpdateProjectRequest,
    ) -> Result<types::ProjectResponse> {
        self.request(
            Method::PUT,
            project_path(tenant, project, &[]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn delete_project(&self, tenant: Uuid, project: Uuid) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            project_path(tenant, project, &[]),
            vec![],
            None,
        )
        .await
    }

    pub async fn list_project_members(
        &self,
        tenant: Uuid,
        project: Uuid,
    ) -> Result<Vec<types::ProjectMemberResponse>> {
        self.request(
            Method::GET,
            project_path(tenant, project, &["members"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn add_project_member(
        &self,
        tenant: Uuid,
        project: Uuid,
        req: &types::AddMemberRequest,
    ) -> Result<types::ProjectMemberResponse> {
        self.request(
            Method::POST,
            project_path(tenant, project, &["members"]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn update_project_member(
        &self,
        tenant: Uuid,
        project: Uuid,
        user: Uuid,
        role: types::ProjectRole,
    ) -> Result<types::ProjectMemberResponse> {
        self.request(
            Method::PUT,
            project_path(tenant, project, &["members", &user.to_string()]),
            vec![],
            body(&types::UpdateMemberRequest { role })?,
        )
        .await
    }

    pub async fn remove_project_member(
        &self,
        tenant: Uuid,
        project: Uuid,
        user: Uuid,
    ) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            project_path(tenant, project, &["members", &user.to_string()]),
            vec![],
            None,
        )
        .await
    }

    // ---- statuses ----

    pub async fn create_status(
        &self,
        tenant: Uuid,
        project: Uuid,
        req: &types::CreateStatusRequest,
    ) -> Result<types::ProjectStatusResponse> {
        self.request(
            Method::POST,
            project_path(tenant, project, &["statuses"]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn update_status(
        &self,
        tenant: Uuid,
        project: Uuid,
        status: Uuid,
        req: &types::UpdateStatusRequest,
    ) -> Result<types::ProjectStatusResponse> {
        self.request(
            Method::PUT,
            project_path(tenant, project, &["statuses", &status.to_string()]),
            vec![],
            body(req)?,
        )
        .await
    }

    /// 使用中のステータスは `migrate_to` へタスクを移してから消す。
    pub async fn delete_status(
        &self,
        tenant: Uuid,
        project: Uuid,
        status: Uuid,
        migrate_to: Option<Uuid>,
    ) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            project_path(tenant, project, &["statuses", &status.to_string()]),
            opt_pairs(&[("migrate_to_status_id", migrate_to.map(|id| id.to_string()))]),
            None,
        )
        .await
    }

    pub async fn reorder_statuses(
        &self,
        tenant: Uuid,
        project: Uuid,
        ids: Vec<Uuid>,
    ) -> Result<Vec<types::ProjectStatusResponse>> {
        self.request(
            Method::PUT,
            project_path(tenant, project, &["statuses", "reorder"]),
            vec![],
            body(&types::ReorderRequest { ids })?,
        )
        .await
    }

    // ---- labels ----

    pub async fn create_label(
        &self,
        tenant: Uuid,
        project: Uuid,
        req: &types::CreateLabelRequest,
    ) -> Result<types::LabelResponse> {
        self.request(
            Method::POST,
            project_path(tenant, project, &["labels"]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn update_label(
        &self,
        tenant: Uuid,
        project: Uuid,
        label: Uuid,
        req: &types::UpdateLabelRequest,
    ) -> Result<types::LabelResponse> {
        self.request(
            Method::PUT,
            project_path(tenant, project, &["labels", &label.to_string()]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn delete_label(&self, tenant: Uuid, project: Uuid, label: Uuid) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            project_path(tenant, project, &["labels", &label.to_string()]),
            vec![],
            None,
        )
        .await
    }

    pub async fn export_labels(&self, tenant: Uuid, project: Uuid) -> Result<types::LabelExport> {
        self.request(
            Method::GET,
            project_path(tenant, project, &["labels", "export"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn import_labels(
        &self,
        tenant: Uuid,
        project: Uuid,
        req: &types::ImportLabelRequest,
    ) -> Result<Vec<types::LabelResponse>> {
        self.request(
            Method::POST,
            project_path(tenant, project, &["labels", "import"]),
            vec![],
            body(req)?,
        )
        .await
    }

    // ---- milestones ----

    pub async fn list_milestones(
        &self,
        tenant: Uuid,
        project: Uuid,
    ) -> Result<Vec<types::MilestoneResponse>> {
        self.request(
            Method::GET,
            project_path(tenant, project, &["milestones"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn get_milestone(
        &self,
        tenant: Uuid,
        project: Uuid,
        milestone: Uuid,
    ) -> Result<types::MilestoneDetail> {
        self.request(
            Method::GET,
            project_path(tenant, project, &["milestones", &milestone.to_string()]),
            vec![],
            None,
        )
        .await
    }

    pub async fn create_milestone(
        &self,
        tenant: Uuid,
        project: Uuid,
        req: &types::CreateMilestoneRequest,
    ) -> Result<types::MilestoneResponse> {
        self.request(
            Method::POST,
            project_path(tenant, project, &["milestones"]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn update_milestone(
        &self,
        tenant: Uuid,
        project: Uuid,
        milestone: Uuid,
        req: &types::UpdateMilestoneRequest,
    ) -> Result<types::MilestoneResponse> {
        self.request(
            Method::PUT,
            project_path(tenant, project, &["milestones", &milestone.to_string()]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn delete_milestone(
        &self,
        tenant: Uuid,
        project: Uuid,
        milestone: Uuid,
    ) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            project_path(tenant, project, &["milestones", &milestone.to_string()]),
            vec![],
            None,
        )
        .await
    }

    // ---- sprints ----

    pub async fn list_sprints(
        &self,
        tenant: Uuid,
        project: Uuid,
        status: Option<types::SprintStatus>,
    ) -> Result<Vec<types::SprintResponse>> {
        self.request(
            Method::GET,
            project_path(tenant, project, &["sprints"]),
            opt_pairs(&[("status", status.map(|s| s.to_string()))]),
            None,
        )
        .await
    }

    pub async fn get_sprint(
        &self,
        tenant: Uuid,
        project: Uuid,
        sprint: Uuid,
    ) -> Result<types::SprintDetail> {
        self.request(
            Method::GET,
            project_path(tenant, project, &["sprints", &sprint.to_string()]),
            vec![],
            None,
        )
        .await
    }

    pub async fn create_sprint(
        &self,
        tenant: Uuid,
        project: Uuid,
        req: &types::CreateSprintRequest,
    ) -> Result<types::SprintResponse> {
        self.request(
            Method::POST,
            project_path(tenant, project, &["sprints"]),
            vec![],
            body(req)?,
        )
        .await
    }

    /// planning のスプリントだけ変更できる（backend が 409 を返す）。
    pub async fn update_sprint(
        &self,
        tenant: Uuid,
        project: Uuid,
        sprint: Uuid,
        req: &types::UpdateSprintRequest,
    ) -> Result<types::SprintResponse> {
        self.request(
            Method::PUT,
            project_path(tenant, project, &["sprints", &sprint.to_string()]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn delete_sprint(&self, tenant: Uuid, project: Uuid, sprint: Uuid) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            project_path(tenant, project, &["sprints", &sprint.to_string()]),
            vec![],
            None,
        )
        .await
    }

    pub async fn start_sprint(
        &self,
        tenant: Uuid,
        project: Uuid,
        sprint: Uuid,
    ) -> Result<types::SprintResponse> {
        self.request(
            Method::POST,
            project_path(tenant, project, &["sprints", &sprint.to_string(), "start"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn complete_sprint(
        &self,
        tenant: Uuid,
        project: Uuid,
        sprint: Uuid,
        req: &types::CompleteSprintRequest,
    ) -> Result<types::SprintResponse> {
        self.request(
            Method::POST,
            project_path(
                tenant,
                project,
                &["sprints", &sprint.to_string(), "complete"],
            ),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn assign_tasks_to_sprint(
        &self,
        tenant: Uuid,
        project: Uuid,
        sprint: Uuid,
        task_ids: Vec<Uuid>,
    ) -> Result<Vec<types::TaskResponse>> {
        self.request(
            Method::POST,
            project_path(tenant, project, &["sprints", &sprint.to_string(), "tasks"]),
            vec![],
            body(&types::AssignTasksRequest { task_ids })?,
        )
        .await
    }

    // ---- custom fields ----

    pub async fn list_custom_fields(
        &self,
        tenant: Uuid,
        project: Uuid,
    ) -> Result<types::CustomFieldListResponse> {
        self.request(
            Method::GET,
            project_path(tenant, project, &["custom-fields"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn create_custom_field(
        &self,
        tenant: Uuid,
        project: Uuid,
        req: &types::CreateCustomFieldRequest,
    ) -> Result<types::ProjectCustomFieldResponse> {
        self.request(
            Method::POST,
            project_path(tenant, project, &["custom-fields"]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn update_custom_field(
        &self,
        tenant: Uuid,
        project: Uuid,
        field: Uuid,
        req: &types::UpdateCustomFieldRequest,
    ) -> Result<types::ProjectCustomFieldResponse> {
        self.request(
            Method::PATCH,
            project_path(tenant, project, &["custom-fields", &field.to_string()]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn delete_custom_field(
        &self,
        tenant: Uuid,
        project: Uuid,
        field: Uuid,
    ) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            project_path(tenant, project, &["custom-fields", &field.to_string()]),
            vec![],
            None,
        )
        .await
    }

    // ---- saved task views ----

    pub async fn list_task_views(
        &self,
        tenant: Uuid,
        project: Uuid,
    ) -> Result<types::TaskViewListResponse> {
        self.request(
            Method::GET,
            project_path(tenant, project, &["task-views"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn create_task_view(
        &self,
        tenant: Uuid,
        project: Uuid,
        req: &types::CreateTaskViewRequest,
    ) -> Result<types::ProjectTaskViewResponse> {
        self.request(
            Method::POST,
            project_path(tenant, project, &["task-views"]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn update_task_view(
        &self,
        tenant: Uuid,
        project: Uuid,
        view: Uuid,
        req: &types::UpdateTaskViewRequest,
    ) -> Result<types::ProjectTaskViewResponse> {
        self.request(
            Method::PATCH,
            project_path(tenant, project, &["task-views", &view.to_string()]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn delete_task_view(&self, tenant: Uuid, project: Uuid, view: Uuid) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            project_path(tenant, project, &["task-views", &view.to_string()]),
            vec![],
            None,
        )
        .await
    }

    // ---- github (read-only + install link) ----

    pub async fn get_github_integration(
        &self,
        tenant: Uuid,
        project: Uuid,
    ) -> Result<types::GithubIntegrationResponse> {
        self.request(
            Method::GET,
            project_path(tenant, project, &["github", "integration"]),
            vec![],
            None,
        )
        .await
    }

    /// GitHub App のインストール URL（ブラウザで開いて連携する）。
    pub async fn start_github_install(
        &self,
        tenant: Uuid,
        project: Uuid,
    ) -> Result<types::GithubInstallUrlResponse> {
        self.request(
            Method::GET,
            project_path(tenant, project, &["github", "install"]),
            vec![],
            None,
        )
        .await
    }

    // ---- tasks ----

    /// ソフト削除。
    pub async fn delete_task(&self, tenant: Uuid, project: Uuid, task: Uuid) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            task_path(tenant, project, task, &[]),
            vec![],
            None,
        )
        .await
    }

    pub async fn archive_task(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<types::TaskResponse> {
        self.request(
            Method::POST,
            task_path(tenant, project, task, &["archive"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn unarchive_task(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<types::TaskResponse> {
        self.request(
            Method::POST,
            task_path(tenant, project, task, &["unarchive"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn bulk_update_tasks(
        &self,
        tenant: Uuid,
        project: Uuid,
        req: &types::BulkUpdateRequest,
    ) -> Result<types::BulkUpdateResponse> {
        self.request(
            Method::POST,
            project_path(tenant, project, &["tasks", "bulk"]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn list_activities(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        cursor: Option<String>,
    ) -> Result<types::ActivityListResponse> {
        self.request(
            Method::GET,
            task_path(tenant, project, task, &["activities"]),
            opt_pairs(&[("limit", Some("50".into())), ("cursor", cursor)]),
            None,
        )
        .await
    }

    pub async fn list_assignees(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<Vec<types::TaskAssigneeResponse>> {
        self.request(
            Method::GET,
            task_path(tenant, project, task, &["assignees"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn add_assignee(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        req: &types::AddAssigneeRequest,
    ) -> Result<types::TaskAssigneeResponse> {
        self.request(
            Method::POST,
            task_path(tenant, project, task, &["assignees"]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn update_assignee(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        user: Uuid,
        role: &str,
    ) -> Result<types::TaskAssigneeResponse> {
        self.request(
            Method::PUT,
            task_path(tenant, project, task, &["assignees", &user.to_string()]),
            vec![],
            body(&types::UpdateAssigneeRequest { role: role.into() })?,
        )
        .await
    }

    pub async fn remove_assignee(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        user: Uuid,
    ) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            task_path(tenant, project, task, &["assignees", &user.to_string()]),
            vec![],
            None,
        )
        .await
    }

    pub async fn list_task_attachments(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<types::TaskAttachmentListResponse> {
        self.request(
            Method::GET,
            task_path(tenant, project, task, &["attachments"]),
            vec![],
            None,
        )
        .await
    }

    /// 添付を外す（ドライブのファイル自体は残る）。
    pub async fn detach_task_file(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        attachment: Uuid,
    ) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            task_path(
                tenant,
                project,
                task,
                &["attachments", &attachment.to_string()],
            ),
            vec![],
            None,
        )
        .await
    }

    /// 投稿者本人のみ。
    pub async fn update_comment(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        comment: Uuid,
        text: &str,
    ) -> Result<types::TaskCommentResponse> {
        self.request(
            Method::PUT,
            task_path(tenant, project, task, &["comments", &comment.to_string()]),
            vec![],
            body(&types::UpdateCommentRequest { body: text.into() })?,
        )
        .await
    }

    /// ソフト削除（投稿者 or テナントオーナー）。
    pub async fn delete_comment(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        comment: Uuid,
    ) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            task_path(tenant, project, task, &["comments", &comment.to_string()]),
            vec![],
            None,
        )
        .await
    }

    pub async fn list_relations(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<types::TaskRelationsResponse> {
        self.request(
            Method::GET,
            task_path(tenant, project, task, &["relations"]),
            vec![],
            None,
        )
        .await
    }

    /// `kind` は API の `type`（例: `blocks` / `blocked_by`）。循環は backend が 409。
    pub async fn add_relation(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        target: Uuid,
        kind: &str,
    ) -> Result<types::TaskRelationResponse> {
        self.request(
            Method::POST,
            task_path(tenant, project, task, &["relations"]),
            vec![],
            body(&types::AddRelationRequest {
                target_task_id: target,
                type_: kind.into(),
            })?,
        )
        .await
    }

    pub async fn remove_relation(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        relation: Uuid,
    ) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            task_path(tenant, project, task, &["relations", &relation.to_string()]),
            vec![],
            None,
        )
        .await
    }

    pub async fn list_time_logs(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<Vec<types::TimeLogResponse>> {
        self.request(
            Method::GET,
            task_path(tenant, project, task, &["time-logs"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn create_time_log(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        req: &types::CreateTimeLogRequest,
    ) -> Result<types::TimeLogResponse> {
        self.request(
            Method::POST,
            task_path(tenant, project, task, &["time-logs"]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn update_time_log(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        log: Uuid,
        req: &types::UpdateTimeLogRequest,
    ) -> Result<types::TimeLogResponse> {
        self.request(
            Method::PUT,
            task_path(tenant, project, task, &["time-logs", &log.to_string()]),
            vec![],
            body(req)?,
        )
        .await
    }

    pub async fn delete_time_log(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
        log: Uuid,
    ) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            task_path(tenant, project, task, &["time-logs", &log.to_string()]),
            vec![],
            None,
        )
        .await
    }

    pub async fn get_time_summary(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<types::TimeLogSummaryResponse> {
        self.request(
            Method::GET,
            task_path(tenant, project, task, &["time-logs", "summary"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn start_timer(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<types::TaskTimerResponse> {
        self.request(
            Method::POST,
            task_path(tenant, project, task, &["timer", "start"]),
            vec![],
            None,
        )
        .await
    }

    /// 止めると作業時間ログが 1 件できる。
    pub async fn stop_timer(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<types::TimeLogResponse> {
        self.request(
            Method::POST,
            task_path(tenant, project, task, &["timer", "stop"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn get_timer_status(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<types::TimerStatusResponse> {
        self.request(
            Method::GET,
            task_path(tenant, project, task, &["timer", "status"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn start_watch(&self, tenant: Uuid, project: Uuid, task: Uuid) -> Result<()> {
        self.request_unit(
            Method::POST,
            task_path(tenant, project, task, &["watch"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn stop_watch(&self, tenant: Uuid, project: Uuid, task: Uuid) -> Result<()> {
        self.request_unit(
            Method::DELETE,
            task_path(tenant, project, task, &["watch"]),
            vec![],
            None,
        )
        .await
    }

    pub async fn list_watchers(
        &self,
        tenant: Uuid,
        project: Uuid,
        task: Uuid,
    ) -> Result<types::WatcherListResponse> {
        self.request(
            Method::GET,
            task_path(tenant, project, task, &["watchers"]),
            vec![],
            None,
        )
        .await
    }
}
