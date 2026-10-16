//! プロジェクト設定。左ナビで区分を切り替え、各区分は表示時に読み込む。
//! 変更は API の結果で一覧を取り直す（並び・既定値は backend が決める）。

mod catalog;
mod integrations;
mod members;
mod overview;
mod planning;

use std::collections::HashSet;
use std::future::Future;

use api::Client;
use api::types::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::date_picker::DatePickerState;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Icon, Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

pub enum ProjectSettingsEvent {
    /// 名前・説明などが変わった（Sidebar の表示を更新する）。
    Updated(ProjectResponse),
    Deleted(Uuid),
    /// ステータス・ラベル・マイルストーン・スプリント・フィールドが変わった。
    CatalogChanged(Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Section {
    Overview,
    Members,
    Statuses,
    Labels,
    Milestones,
    Sprints,
    Fields,
    Notifications,
    Github,
}

impl Section {
    const ALL: [Section; 9] = [
        Section::Overview,
        Section::Members,
        Section::Statuses,
        Section::Labels,
        Section::Milestones,
        Section::Sprints,
        Section::Fields,
        Section::Notifications,
        Section::Github,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::Overview => t!("projects.section.overview"),
            Section::Members => t!("projects.section.members"),
            Section::Statuses => t!("projects.section.statuses"),
            Section::Labels => t!("projects.section.labels"),
            Section::Milestones => t!("projects.section.milestones"),
            Section::Sprints => t!("projects.section.sprints"),
            Section::Fields => t!("projects.section.fields"),
            Section::Notifications => t!("projects.section.notifications"),
            Section::Github => t!("projects.section.github"),
        }
    }

    fn icon(self) -> IconName {
        match self {
            Section::Overview => IconName::Info,
            Section::Members => IconName::Users,
            Section::Statuses => IconName::ListTodo,
            Section::Labels => IconName::Tag,
            Section::Milestones => IconName::Flag,
            Section::Sprints => IconName::Calendar,
            Section::Fields => IconName::LayoutDashboard,
            Section::Notifications => IconName::Bell,
            Section::Github => IconName::Github,
        }
    }
}

/// 入力欄（作成・編集フォーム）。編集中の id があれば保存は更新になる。
pub(crate) struct Forms {
    pub name: Entity<InputState>,
    pub emoji: Entity<InputState>,
    pub description: Entity<TextareaState>,
    pub delete_confirm: Entity<InputState>,
    pub status_name: Entity<InputState>,
    pub status_color: Entity<InputState>,
    pub status_done: bool,
    pub editing_status: Option<Uuid>,
    pub label_name: Entity<InputState>,
    pub label_color: Entity<InputState>,
    pub label_description: Entity<InputState>,
    pub editing_label: Option<Uuid>,
    pub milestone_name: Entity<InputState>,
    pub milestone_description: Entity<InputState>,
    pub milestone_due: Entity<DatePickerState>,
    pub editing_milestone: Option<Uuid>,
    pub sprint_name: Entity<InputState>,
    pub sprint_goal: Entity<InputState>,
    pub sprint_start: Entity<DatePickerState>,
    pub sprint_end: Entity<DatePickerState>,
    pub editing_sprint: Option<Uuid>,
    pub field_name: Entity<InputState>,
    pub field_options: Entity<InputState>,
    pub field_type: CustomFieldType,
    pub field_required: bool,
    pub editing_field: Option<Uuid>,
}

pub struct ProjectSettingsView {
    pub(crate) client: Option<Client>,
    pub(crate) tenant: Option<Uuid>,
    pub(crate) project: Option<Uuid>,
    pub(crate) info: Option<ProjectResponse>,
    section: Section,
    /// 追加フォームを開いているか（区分を移ると閉じる）。編集中は常に開く。
    pub(crate) form_open: bool,
    generation: u64,
    /// 実行中のリクエスト数（ボタンの無効化に使う）。
    pub(crate) busy: usize,
    loaded: HashSet<Section>,
    /// render で通知に出す文言（spawn 側は Window に触れないため貯める）。
    messages: Vec<String>,
    /// render で入力欄を空にする区分（成功後のリセット）。
    reset: HashSet<Section>,
    /// render で概要の入力欄を `info` に合わせる。
    sync_overview: bool,
    pub(crate) members: Vec<ProjectMemberResponse>,
    pub(crate) tenant_members: Vec<TenantMemberResponse>,
    pub(crate) statuses: Vec<ProjectStatusResponse>,
    pub(crate) labels: Vec<LabelResponse>,
    pub(crate) milestones: Vec<MilestoneDetail>,
    pub(crate) sprints: Vec<SprintDetail>,
    pub(crate) fields: Vec<ProjectCustomFieldResponse>,
    pub(crate) notification_settings: Option<NotificationSettingsResponse>,
    pub(crate) github: Option<GithubIntegrationResponse>,
    pub(crate) forms: Forms,
    _subs: Vec<Subscription>,
}

impl EventEmitter<ProjectSettingsEvent> for ProjectSettingsView {}

impl ProjectSettingsView {
    pub fn new(
        client: Option<Client>,
        tenant: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut input = |placeholder: &'static str| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let forms = Forms {
            name: input(t!("projects.overview.name_ph")),
            emoji: input(t!("projects.overview.emoji_ph")),
            delete_confirm: input(t!("projects.overview.delete_ph")),
            status_name: input(t!("projects.statuses.name_ph")),
            status_color: input("#1f6feb"),
            status_done: false,
            editing_status: None,
            label_name: input(t!("projects.labels.name_ph")),
            label_color: input("#1f6feb"),
            label_description: input(t!("projects.labels.description_ph")),
            editing_label: None,
            milestone_name: input(t!("projects.milestones.name_ph")),
            milestone_description: input(t!("projects.milestones.description_ph")),
            editing_milestone: None,
            sprint_name: input(t!("projects.sprints.name_ph")),
            sprint_goal: input(t!("projects.sprints.goal_ph")),
            editing_sprint: None,
            field_name: input(t!("projects.fields.name_ph")),
            field_options: input(t!("projects.fields.options_ph")),
            field_type: CustomFieldType::Text,
            field_required: false,
            editing_field: None,
            description: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder(t!("projects.overview.description_ph"))
            }),
            milestone_due: cx.new(|cx| DatePickerState::new(window, cx).date_format("%Y-%m-%d")),
            sprint_start: cx.new(|cx| DatePickerState::new(window, cx).date_format("%Y-%m-%d")),
            sprint_end: cx.new(|cx| DatePickerState::new(window, cx).date_format("%Y-%m-%d")),
        };
        type Save = fn(&mut ProjectSettingsView, &mut Context<ProjectSettingsView>);
        // Enter で保存。入力のたびに描き直す（色の見本・削除確認の有効化のため）。
        let bindings: [(&Entity<InputState>, Option<Save>); 14] = [
            (&forms.name, Some(Self::save_overview)),
            (&forms.emoji, Some(Self::save_overview)),
            (&forms.delete_confirm, None),
            (&forms.status_name, Some(Self::save_status)),
            (&forms.status_color, Some(Self::save_status)),
            (&forms.label_name, Some(Self::save_label)),
            (&forms.label_color, Some(Self::save_label)),
            (&forms.label_description, Some(Self::save_label)),
            (&forms.milestone_name, Some(Self::save_milestone)),
            (&forms.milestone_description, Some(Self::save_milestone)),
            (&forms.sprint_name, Some(Self::save_sprint)),
            (&forms.sprint_goal, Some(Self::save_sprint)),
            (&forms.field_name, Some(Self::save_field)),
            (&forms.field_options, Some(Self::save_field)),
        ];
        let subs = bindings
            .into_iter()
            .map(|(input, save)| {
                cx.subscribe(input, move |this, _, ev: &InputEvent, cx| match ev {
                    InputEvent::PressEnter { .. } => {
                        if let Some(save) = save {
                            save(this, cx);
                        }
                    }
                    InputEvent::Change => cx.notify(),
                    _ => {}
                })
            })
            .collect();
        Self {
            _subs: subs,
            client,
            tenant,
            project: None,
            info: None,
            section: Section::Overview,
            form_open: false,
            generation: 0,
            busy: 0,
            loaded: HashSet::new(),
            messages: vec![],
            reset: HashSet::new(),
            sync_overview: false,
            members: vec![],
            tenant_members: vec![],
            statuses: vec![],
            labels: vec![],
            milestones: vec![],
            sprints: vec![],
            fields: vec![],
            notification_settings: None,
            github: None,
            forms,
        }
    }

    pub fn set_client(&mut self, client: Client, tenant: Option<Uuid>, cx: &mut Context<Self>) {
        if self.tenant != tenant {
            self.clear_loaded();
            self.project = None;
        }
        self.client = Some(client);
        self.tenant = tenant;
        cx.notify();
    }

    pub fn clear_client(&mut self, cx: &mut Context<Self>) {
        self.client = None;
        self.project = None;
        self.clear_loaded();
        cx.notify();
    }

    pub(crate) fn clear_loaded(&mut self) {
        self.generation += 1;
        self.busy = 0;
        self.info = None;
        self.loaded.clear();
        self.members.clear();
        self.tenant_members.clear();
        self.statuses.clear();
        self.labels.clear();
        self.milestones.clear();
        self.sprints.clear();
        self.fields.clear();
        self.notification_settings = None;
        self.github = None;
        self.reset.extend(Section::ALL);
        let f = &mut self.forms;
        f.editing_status = None;
        f.editing_label = None;
        f.editing_milestone = None;
        f.editing_sprint = None;
        f.editing_field = None;
    }

    /// 表示するプロジェクトを切り替え、概要と現在の区分を読み込む。
    pub fn open(&mut self, project: Uuid, cx: &mut Context<Self>) {
        if self.project != Some(project) {
            self.clear_loaded();
            self.project = Some(project);
            self.section = Section::Overview;
        }
        self.load(Section::Overview, cx);
        if self.section != Section::Overview {
            self.load(self.section, cx);
        }
    }

    fn select(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        self.form_open = false;
        if !self.loaded.contains(&section) {
            self.load(section, cx);
        }
        cx.notify();
    }

    pub(crate) fn ctx(&self) -> Option<(Client, Uuid, Uuid)> {
        Some((self.client.clone()?, self.tenant?, self.project?))
    }

    /// API を 1 つ呼び、成功したら `done`、失敗したら通知に出す。
    /// プロジェクトやテナントが変わった後に返った結果は捨てる。
    pub(crate) fn call<T, F>(
        &mut self,
        cx: &mut Context<Self>,
        fut: F,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) where
        T: 'static,
        F: Future<Output = api::Result<T>> + 'static,
    {
        let generation = self.generation;
        self.busy += 1;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = fut.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = this.busy.saturating_sub(1);
                match res {
                    Ok(value) => done(this, value, cx),
                    Err(e) => this.messages.push(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn notice(&mut self, text: impl Into<String>) {
        self.messages.push(text.into());
    }

    pub(crate) fn reset_form(&mut self, section: Section) {
        self.reset.insert(section);
        self.form_open = false;
    }

    /// 区分のページで追加フォームが何番目の要素か（フォームの無い区分は None）。
    fn form_index(section: Section) -> Option<usize> {
        match section {
            Section::Statuses | Section::Milestones | Section::Sprints | Section::Fields => Some(0),
            Section::Labels => Some(1),
            _ => None,
        }
    }

    /// 追加・編集フォームを出すか。
    pub(crate) fn form_shown(&self, section: Section) -> bool {
        let f = &self.forms;
        self.form_open
            || match section {
                Section::Statuses => f.editing_status.is_some(),
                Section::Labels => f.editing_label.is_some(),
                Section::Milestones => f.editing_milestone.is_some(),
                Section::Sprints => f.editing_sprint.is_some(),
                Section::Fields => f.editing_field.is_some(),
                _ => false,
            }
    }

    /// 一覧系の変更を shell へ知らせる（タスク一覧の候補を取り直してもらう）。
    pub(crate) fn catalog_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(project) = self.project {
            cx.emit(ProjectSettingsEvent::CatalogChanged(project));
        }
    }

    pub(crate) fn load(&mut self, section: Section, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        match section {
            Section::Overview => self.call(
                cx,
                async move { client.get_project(tenant, project).await },
                |this, info, _| {
                    this.info = Some(info);
                    this.sync_overview = true;
                    this.loaded.insert(Section::Overview);
                },
            ),
            Section::Members => {
                let c = client.clone();
                self.call(
                    cx,
                    async move { c.list_project_members(tenant, project).await },
                    |this, members, _| {
                        this.members = members;
                        this.loaded.insert(Section::Members);
                    },
                );
                // 追加候補。権限が無ければ空のまま（追加ボタンを出さない）。
                cx.spawn(async move |this, cx| {
                    if let Ok(list) = client.list_tenant_members(tenant).await {
                        let _ = this.update(cx, |this, cx| {
                            if this.tenant == Some(tenant) {
                                this.tenant_members = list;
                                cx.notify();
                            }
                        });
                    }
                })
                .detach();
            }
            Section::Statuses => self.call(
                cx,
                async move { client.list_statuses(tenant, project).await },
                |this, mut statuses, _| {
                    statuses.sort_by_key(|s| s.position);
                    this.statuses = statuses;
                    this.loaded.insert(Section::Statuses);
                },
            ),
            Section::Labels => self.call(
                cx,
                async move { client.list_labels(tenant, project).await },
                |this, labels, _| {
                    this.labels = labels;
                    this.loaded.insert(Section::Labels);
                },
            ),
            Section::Milestones => self.call(
                cx,
                async move {
                    // 一覧に進捗が無いので 1 件ずつ詳細を取る。
                    let mut out = vec![];
                    for m in client.list_milestones(tenant, project).await? {
                        out.push(client.get_milestone(tenant, project, m.id).await?);
                    }
                    Ok(out)
                },
                |this, mut milestones, _| {
                    milestones.sort_by(|a, b| a.due_date.cmp(&b.due_date));
                    this.milestones = milestones;
                    this.loaded.insert(Section::Milestones);
                },
            ),
            Section::Sprints => self.call(
                cx,
                async move {
                    let mut out = vec![];
                    for s in client.list_sprints(tenant, project, None).await? {
                        out.push(client.get_sprint(tenant, project, s.id).await?);
                    }
                    Ok(out)
                },
                |this, mut sprints, _| {
                    sprints.sort_by(|a, b| a.start_date.cmp(&b.start_date));
                    this.sprints = sprints;
                    this.loaded.insert(Section::Sprints);
                },
            ),
            Section::Fields => self.call(
                cx,
                async move { client.list_custom_fields(tenant, project).await },
                |this, list, _| {
                    let mut fields = list.fields;
                    fields.sort_by_key(|f| f.position);
                    this.fields = fields;
                    this.loaded.insert(Section::Fields);
                },
            ),
            Section::Notifications => self.call(
                cx,
                async move { client.get_notification_settings(project).await },
                |this, settings, _| {
                    this.notification_settings = Some(settings);
                    this.loaded.insert(Section::Notifications);
                },
            ),
            Section::Github => self.call(
                cx,
                async move { client.get_github_integration(tenant, project).await },
                |this, github, _| {
                    this.github = Some(github);
                    this.loaded.insert(Section::Github);
                },
            ),
        }
    }

    /// spawn 側から積まれた入力欄の更新を Window のある render で適用する。
    fn apply_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for message in self.messages.drain(..) {
            window.push_notification(Notification::new().message(message), cx);
        }
        if std::mem::take(&mut self.sync_overview)
            && let Some(info) = self.info.clone()
        {
            let f = &self.forms;
            f.name
                .update(cx, |s, cx| s.set_value(info.name.clone(), window, cx));
            f.emoji.update(cx, |s, cx| {
                s.set_value(info.icon_emoji.clone().unwrap_or_default(), window, cx)
            });
            f.description.update(cx, |s, cx| {
                s.set_value(info.description.clone(), window, cx)
            });
            f.delete_confirm
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        for section in std::mem::take(&mut self.reset) {
            self.clear_form(section, window, cx);
        }
    }

    pub(crate) fn clear_form(&mut self, section: Section, window: &mut Window, cx: &mut App) {
        let f = &mut self.forms;
        let mut clear = |input: &Entity<InputState>, cx: &mut App| {
            input.update(cx, |s, cx| s.set_value("", window, cx))
        };
        match section {
            Section::Statuses => {
                f.editing_status = None;
                f.status_done = false;
                clear(&f.status_name, cx);
                clear(&f.status_color, cx);
            }
            Section::Labels => {
                f.editing_label = None;
                clear(&f.label_name, cx);
                clear(&f.label_color, cx);
                clear(&f.label_description, cx);
            }
            Section::Milestones => {
                f.editing_milestone = None;
                clear(&f.milestone_name, cx);
                clear(&f.milestone_description, cx);
                f.milestone_due.update(cx, |s, cx| {
                    s.set_date(
                        gpui_kit::component::calendar::Date::Single(None),
                        window,
                        cx,
                    )
                });
            }
            Section::Sprints => {
                f.editing_sprint = None;
                clear(&f.sprint_name, cx);
                clear(&f.sprint_goal, cx);
                for picker in [&f.sprint_start, &f.sprint_end] {
                    picker.update(cx, |s, cx| {
                        s.set_date(
                            gpui_kit::component::calendar::Date::Single(None),
                            window,
                            cx,
                        )
                    });
                }
            }
            Section::Fields => {
                f.editing_field = None;
                f.field_required = false;
                f.field_type = CustomFieldType::Text;
                clear(&f.field_name, cx);
                clear(&f.field_options, cx);
            }
            Section::Overview | Section::Members | Section::Notifications | Section::Github => {}
        }
    }

    pub(crate) fn is_loading(&self, section: Section) -> bool {
        !self.loaded.contains(&section) && self.busy > 0
    }
}

impl Render for ProjectSettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.apply_pending(window, cx);
        let c = ::ui::colors(cx);
        if self.project.is_none() || self.client.is_none() {
            return div()
                .id("project-settings")
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(c.text_muted)
                .child(t!("projects.select_project"))
                .into_any_element();
        }
        let section = self.section;
        let mut body: Vec<AnyElement> = match section {
            Section::Overview => self.overview_page(&c, cx),
            Section::Members => self.members_page(&c, cx),
            Section::Statuses => self.statuses_page(&c, cx),
            Section::Labels => self.labels_page(&c, cx),
            Section::Milestones => self.milestones_page(&c, cx),
            Section::Sprints => self.sprints_page(&c, cx),
            Section::Fields => self.fields_page(&c, cx),
            Section::Notifications => self.notifications_page(&c, cx),
            Section::Github => self.github_page(&c, cx),
        };
        // 追加フォームは常に出さず、見出しの「追加」で開く（一覧を先に見せる）。
        let form_ix = Self::form_index(section);
        let show_form = self.form_shown(section);
        if let Some(ix) = form_ix.filter(|ix| !show_form && *ix < body.len()) {
            body.remove(ix);
        }
        let nav = div()
            .w(px(200.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_0p5()
            .p_2()
            .border_r_1()
            .border_color(c.border)
            .children(Section::ALL.into_iter().map(|item| {
                ::ui::nav_item(
                    &c,
                    SharedString::from(format!("project-nav-{item:?}")),
                    Icon::new(item.icon()).small(),
                    item.label(),
                    item == section,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.select(item, cx)))
            }));

        let content = div()
            .id("project-settings-scroll")
            .flex_1()
            .h_full()
            .min_w_0()
            .overflow_y_scroll()
            .child(
                div()
                    .max_w(px(760.))
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .px_6()
                    .py_5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .pb_1()
                            .child(
                                div()
                                    .flex_1()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(section.label()),
                            )
                            .when(form_ix.is_some() && !show_form, |d| {
                                d.child(
                                    Button::new("section-add")
                                        .small()
                                        .icon(IconName::Plus)
                                        .label(t!("projects.common.add"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.form_open = true;
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .children(body),
            );

        div()
            .id("project-settings")
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_row()
            .child(nav)
            .child(content)
            .into_any_element()
    }
}
