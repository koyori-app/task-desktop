//! ステータス・ラベル・カスタムフィールド（タスクに付ける候補の定義）。

use api::types::{
    CreateCustomFieldRequest, CreateLabelRequest, CreateStatusRequest, CustomFieldType,
    ImportLabelRequest, LabelExport, UpdateCustomFieldRequest, UpdateLabelRequest,
    UpdateStatusRequest,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

use super::{ProjectSettingsView, Section};
use crate::ui::{
    Colors, PRESET_COLORS, badge, card, confirm, field, parse_hex, placeholder, row, swatch,
};

const DEFAULT_COLOR: &str = "#8b949e";

const FIELD_TYPES: [CustomFieldType; 6] = [
    CustomFieldType::Text,
    CustomFieldType::Number,
    CustomFieldType::Select,
    CustomFieldType::Date,
    CustomFieldType::Url,
    CustomFieldType::Checkbox,
];

pub(crate) fn field_type_label(t: CustomFieldType) -> &'static str {
    match t {
        CustomFieldType::Text => t!("projects.fields.type_text"),
        CustomFieldType::Number => t!("projects.fields.type_number"),
        CustomFieldType::Select => t!("projects.fields.type_select"),
        CustomFieldType::Date => t!("projects.fields.type_date"),
        CustomFieldType::Url => t!("projects.fields.type_url"),
        CustomFieldType::Checkbox => t!("projects.fields.type_checkbox"),
    }
}

/// 「a, b, c」→ select の options（`{label, value}` の配列）。空要素・重複は除く。
/// 既存の options と value が一致するものは label を引き継ぐ（Web と同じ）。
pub(crate) fn parse_options(text: &str, existing: Option<&serde_json::Value>) -> serde_json::Value {
    let label_of = |value: &str| {
        existing
            .and_then(|e| e.as_array())
            .and_then(|items| {
                items.iter().find_map(|o| {
                    (o.get("value")?.as_str()? == value)
                        .then(|| o.get("label")?.as_str().map(str::to_string))
                        .flatten()
                })
            })
            .unwrap_or_else(|| value.to_string())
    };
    let mut seen = std::collections::HashSet::new();
    serde_json::Value::Array(
        text.split([',', '\n'])
            .map(str::trim)
            .filter(|v| !v.is_empty() && seen.insert(v.to_string()))
            .map(|v| serde_json::json!({ "label": label_of(v), "value": v }))
            .collect(),
    )
}

pub(crate) fn serialize_options(options: Option<&serde_json::Value>) -> String {
    options
        .and_then(|o| o.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|o| o.get("value")?.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

fn color_or_default(input: &Entity<InputState>, cx: &App) -> String {
    let value = input.read(cx).value().trim().to_string();
    if parse_hex(&value).is_some() {
        value
    } else {
        DEFAULT_COLOR.into()
    }
}

impl ProjectSettingsView {
    /// 色の入力欄 + 見本 + 候補の色。
    fn color_field(
        &self,
        c: &Colors,
        id: &'static str,
        input: &Entity<InputState>,
        cx: &App,
    ) -> Div {
        let preview = parse_hex(&input.read(cx).value()).unwrap_or(c.text_muted);
        let input_for_presets = input.clone();
        field(
            c,
            t!("projects.common.color"),
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(swatch(preview).size(px(20.)))
                .child(div().w(px(110.)).child(Input::new(input)))
                .children(PRESET_COLORS.iter().enumerate().map(move |(ix, hex)| {
                    let input = input_for_presets.clone();
                    let color = parse_hex(hex).unwrap_or(preview);
                    div()
                        .id(SharedString::from(format!("{id}-preset-{ix}")))
                        .size(px(16.))
                        .rounded_full()
                        .bg(color)
                        .cursor_pointer()
                        .on_click(move |_, window, cx| {
                            input.update(cx, |s, cx| s.set_value(*hex, window, cx));
                        })
                })),
        )
    }

    // ---- statuses ----

    pub(crate) fn save_status(&mut self, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        let f = &self.forms;
        let name = f.status_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.notice(t!("projects.common.name_required"));
            cx.notify();
            return;
        }
        let color = color_or_default(&f.status_color, cx);
        let done = f.status_done;
        let editing = f.editing_status;
        let position = self.statuses.iter().map(|s| s.position).max().unwrap_or(-1) + 1;
        self.call(
            cx,
            async move {
                match editing {
                    Some(id) => {
                        let req = UpdateStatusRequest {
                            name: Some(name),
                            color: Some(color),
                            is_done_state: Some(done),
                            ..Default::default()
                        };
                        client.update_status(tenant, project, id, &req).await
                    }
                    None => {
                        let req = CreateStatusRequest {
                            name,
                            color,
                            is_done_state: Some(done),
                            is_default: None,
                            is_default_done: None,
                            position,
                        };
                        client.create_status(tenant, project, &req).await
                    }
                }
            },
            |this, _, cx| this.after_catalog_change(Section::Statuses, cx),
        );
    }

    fn edit_status(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.statuses.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        let f = &mut self.forms;
        f.editing_status = Some(id);
        f.status_done = s.is_done_state;
        f.status_name
            .update(cx, |i, cx| i.set_value(s.name.clone(), window, cx));
        f.status_color
            .update(cx, |i, cx| i.set_value(s.color.clone(), window, cx));
        cx.notify();
    }

    fn update_status_flags(&mut self, id: Uuid, req: UpdateStatusRequest, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        self.call(
            cx,
            async move { client.update_status(tenant, project, id, &req).await },
            |this, _, cx| this.after_catalog_change(Section::Statuses, cx),
        );
    }

    fn move_status(&mut self, id: Uuid, delta: isize, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        let mut ids: Vec<Uuid> = self.statuses.iter().map(|s| s.id).collect();
        let Some(ix) = ids.iter().position(|x| *x == id) else {
            return;
        };
        let target = ix as isize + delta;
        if target < 0 || target as usize >= ids.len() {
            return;
        }
        ids.swap(ix, target as usize);
        self.call(
            cx,
            async move { client.reorder_statuses(tenant, project, ids).await },
            |this, mut statuses, cx| {
                statuses.sort_by_key(|s| s.position);
                this.statuses = statuses;
                this.catalog_changed(cx);
            },
        );
    }

    fn delete_status(&mut self, id: Uuid, migrate_to: Option<Uuid>, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        self.call(
            cx,
            async move { client.delete_status(tenant, project, id, migrate_to).await },
            |this, (), cx| this.after_catalog_change(Section::Statuses, cx),
        );
    }

    /// 作成・更新・削除の後: 入力欄を戻し、一覧を取り直して shell に知らせる。
    fn after_catalog_change(&mut self, section: Section, cx: &mut Context<Self>) {
        self.reset_form(section);
        self.load(section, cx);
        self.catalog_changed(cx);
    }

    pub(crate) fn form_buttons(
        &self,
        id: &'static str,
        editing: bool,
        cx: &mut Context<Self>,
        save: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        section: Section,
    ) -> Div {
        let busy = self.busy > 0;
        div()
            .flex()
            .justify_end()
            .gap_2()
            .when(editing || self.form_open, |d| {
                d.child(
                    Button::new(SharedString::from(format!("{id}-cancel")))
                        .small()
                        .ghost()
                        .label(t!("projects.common.cancel"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.clear_form(section, window, cx);
                            this.form_open = false;
                            cx.notify();
                        })),
                )
            })
            .child(
                Button::new(SharedString::from(format!("{id}-save")))
                    .small()
                    .primary()
                    .disabled(busy)
                    .icon(if editing {
                        IconName::Check
                    } else {
                        IconName::Plus
                    })
                    .label(if editing {
                        t!("projects.common.save")
                    } else {
                        t!("projects.common.add")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| save(this, cx))),
            )
    }

    pub(crate) fn statuses_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let f = &self.forms;
        let busy = self.busy > 0;
        let editing = f.editing_status.is_some();
        let form = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(if editing {
                        t!("projects.statuses.edit")
                    } else {
                        t!("projects.statuses.new")
                    }),
            )
            .child(field(
                c,
                t!("projects.common.name"),
                Input::new(&f.status_name),
            ))
            .child(self.color_field(c, "status-color", &f.status_color, cx))
            .child(
                Switch::new("status-done")
                    .checked(f.status_done)
                    .label(t!("projects.statuses.done_state"))
                    .on_click(cx.listener(|this, v: &bool, _, cx| {
                        this.forms.status_done = *v;
                        cx.notify();
                    })),
            )
            .child(self.form_buttons(
                "status",
                editing,
                cx,
                |this, cx| this.save_status(cx),
                Section::Statuses,
            ))
            .into_any_element();

        let mut rows = vec![];
        if self.is_loading(Section::Statuses) {
            rows.push(placeholder(c, t!("projects.common.loading")));
        } else if self.statuses.is_empty() {
            rows.push(placeholder(c, t!("projects.statuses.empty")));
        }
        let this = cx.entity().downgrade();
        let count = self.statuses.len();
        for (ix, s) in self.statuses.iter().enumerate() {
            let id = s.id;
            let others: Vec<(Uuid, String)> = self
                .statuses
                .iter()
                .filter(|o| o.id != id)
                .map(|o| (o.id, o.name.clone()))
                .collect();
            let (is_default, is_done, is_default_done) =
                (s.is_default, s.is_done_state, s.is_default_done);
            let name = s.name.clone();
            let menu_owner = this.clone();
            let more = Button::new(SharedString::from(format!("status-more-{id}")))
                .ghost()
                .small()
                .icon(IconName::EllipsisVertical)
                .disabled(busy)
                .dropdown_menu(move |mut menu, _, _| {
                    let owner = menu_owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(t!("projects.statuses.set_default"))
                            .checked(is_default)
                            .disabled(is_default)
                            .on_click(move |_, _, cx| {
                                let req = UpdateStatusRequest {
                                    is_default: Some(true),
                                    ..Default::default()
                                };
                                let _ = owner
                                    .update(cx, |this, cx| this.update_status_flags(id, req, cx));
                            }),
                    );
                    if is_done {
                        let owner = menu_owner.clone();
                        menu = menu.item(
                            PopupMenuItem::new(t!("projects.statuses.set_default_done"))
                                .checked(is_default_done)
                                .disabled(is_default_done)
                                .on_click(move |_, _, cx| {
                                    let req = UpdateStatusRequest {
                                        is_default_done: Some(true),
                                        ..Default::default()
                                    };
                                    let _ = owner.update(cx, |this, cx| {
                                        this.update_status_flags(id, req, cx)
                                    });
                                }),
                        );
                    }
                    menu = menu.separator();
                    // 使用中のステータスはタスクの移し先が要る。
                    for (target, target_name) in &others {
                        let (target, owner) = (*target, menu_owner.clone());
                        let (name, target_name) = (name.clone(), target_name.clone());
                        menu = menu.item(
                            PopupMenuItem::new(t!(
                                "projects.statuses.delete_move",
                                name = target_name
                            ))
                            .on_click(move |_, window, cx| {
                                let owner = owner.clone();
                                confirm(
                                    window,
                                    cx,
                                    t!("projects.statuses.delete_title", name = name),
                                    t!("projects.statuses.delete_desc", target = target_name),
                                    move |_, cx| {
                                        let _ = owner.update(cx, |this, cx| {
                                            this.delete_status(id, Some(target), cx)
                                        });
                                    },
                                );
                            }),
                        );
                    }
                    menu
                });
            let edit_owner = this.clone();
            rows.push(row(
                c,
                Some(swatch(parse_hex(&s.color).unwrap_or(c.text_muted)).into_any_element()),
                s.name.clone(),
                None,
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .when(is_default, |d| {
                        d.child(badge(c, t!("projects.statuses.default"), Some(c.accent)))
                    })
                    .when(is_done, |d| {
                        d.child(badge(c, t!("projects.statuses.done"), Some(c.success)))
                    })
                    .when(is_default_done, |d| {
                        d.child(badge(
                            c,
                            t!("projects.statuses.default_done"),
                            Some(c.success),
                        ))
                    })
                    .child(
                        Button::new(SharedString::from(format!("status-up-{id}")))
                            .ghost()
                            .small()
                            .icon(IconName::ArrowUp)
                            .tooltip(t!("projects.common.move_up"))
                            .disabled(busy || ix == 0)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.move_status(id, -1, cx)),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("status-down-{id}")))
                            .ghost()
                            .small()
                            .icon(IconName::ArrowDown)
                            .tooltip(t!("projects.common.move_down"))
                            .disabled(busy || ix + 1 == count)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.move_status(id, 1, cx)),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("status-edit-{id}")))
                            .ghost()
                            .small()
                            .label(t!("projects.common.edit"))
                            .on_click(move |_, window, cx| {
                                let _ = edit_owner
                                    .update(cx, |this, cx| this.edit_status(id, window, cx));
                            }),
                    )
                    .child(more),
            ));
        }
        vec![card(c, vec![form]), card(c, rows)]
    }

    // ---- labels ----

    pub(crate) fn save_label(&mut self, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        let f = &self.forms;
        let name = f.label_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.notice(t!("projects.common.name_required"));
            cx.notify();
            return;
        }
        let color = color_or_default(&f.label_color, cx);
        let description = f.label_description.read(cx).value().trim().to_string();
        let editing = f.editing_label;
        self.call(
            cx,
            async move {
                match editing {
                    Some(id) => {
                        let req = UpdateLabelRequest {
                            name: Some(name),
                            color: Some(color),
                            description: Some(description),
                            ..Default::default()
                        };
                        client.update_label(tenant, project, id, &req).await
                    }
                    None => {
                        let req = CreateLabelRequest {
                            name,
                            color,
                            description: Some(description),
                            icon_url: None,
                        };
                        client.create_label(tenant, project, &req).await
                    }
                }
            },
            |this, _, cx| this.after_catalog_change(Section::Labels, cx),
        );
    }

    fn edit_label(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(l) = self.labels.iter().find(|l| l.id == id).cloned() else {
            return;
        };
        let f = &mut self.forms;
        f.editing_label = Some(id);
        f.label_name
            .update(cx, |i, cx| i.set_value(l.name.clone(), window, cx));
        f.label_color
            .update(cx, |i, cx| i.set_value(l.color.clone(), window, cx));
        f.label_description
            .update(cx, |i, cx| i.set_value(l.description.clone(), window, cx));
        cx.notify();
    }

    fn delete_label(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        self.call(
            cx,
            async move { client.delete_label(tenant, project, id).await },
            |this, (), cx| this.after_catalog_change(Section::Labels, cx),
        );
    }

    fn export_labels(&mut self, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        self.call(
            cx,
            async move { client.export_labels(tenant, project).await },
            |this, export, cx| match serde_json::to_string_pretty(&export) {
                Ok(json) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(json));
                    this.notice(t!("projects.labels.exported", count = export.labels.len()));
                }
                Err(e) => this.notice(e.to_string()),
            },
        );
    }

    /// クリップボードの JSON（エクスポートと同じ形）を取り込む。同名は飛ばす。
    fn import_labels(&mut self, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        let text = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default();
        let export: LabelExport = match serde_json::from_str(&text) {
            Ok(export) => export,
            Err(_) => {
                self.notice(t!("projects.labels.import_invalid"));
                cx.notify();
                return;
            }
        };
        let req = ImportLabelRequest {
            labels: export.labels,
            version: export.version,
            on_conflict: Some(api::types::ImportConflict::Skip),
        };
        self.call(
            cx,
            async move { client.import_labels(tenant, project, &req).await },
            |this, labels, cx| {
                this.notice(t!("projects.labels.imported", count = labels.len()));
                this.after_catalog_change(Section::Labels, cx);
            },
        );
    }

    pub(crate) fn labels_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let f = &self.forms;
        let busy = self.busy > 0;
        let editing = f.editing_label.is_some();
        let toolbar = div()
            .flex()
            .justify_end()
            .gap_2()
            .child(
                Button::new("labels-export")
                    .outline()
                    .small()
                    .icon(IconName::Copy)
                    .label(t!("projects.labels.export"))
                    .tooltip(t!("projects.labels.export_desc"))
                    .disabled(busy || self.labels.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| this.export_labels(cx))),
            )
            .child(
                Button::new("labels-import")
                    .outline()
                    .small()
                    .icon(IconName::ClipboardPaste)
                    .label(t!("projects.labels.import"))
                    .tooltip(t!("projects.labels.import_desc"))
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.import_labels(cx))),
            )
            .into_any_element();
        let form = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(if editing {
                        t!("projects.labels.edit")
                    } else {
                        t!("projects.labels.new")
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(div().flex_1().child(field(
                        c,
                        t!("projects.common.name"),
                        Input::new(&f.label_name),
                    )))
                    .child(div().flex_1().child(field(
                        c,
                        t!("projects.common.description"),
                        Input::new(&f.label_description),
                    ))),
            )
            .child(self.color_field(c, "label-color", &f.label_color, cx))
            .child(self.form_buttons(
                "label",
                editing,
                cx,
                |this, cx| this.save_label(cx),
                Section::Labels,
            ))
            .into_any_element();

        let mut rows = vec![];
        if self.is_loading(Section::Labels) {
            rows.push(placeholder(c, t!("projects.common.loading")));
        } else if self.labels.is_empty() {
            rows.push(placeholder(c, t!("projects.labels.empty")));
        }
        let this = cx.entity().downgrade();
        for l in &self.labels {
            let id = l.id;
            let name = l.name.clone();
            let (edit_owner, delete_owner) = (this.clone(), this.clone());
            rows.push(row(
                c,
                Some(swatch(parse_hex(&l.color).unwrap_or(c.text_muted)).into_any_element()),
                l.name.clone(),
                Some(l.description.clone()),
                div()
                    .flex()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("label-edit-{id}")))
                            .ghost()
                            .small()
                            .label(t!("projects.common.edit"))
                            .on_click(move |_, window, cx| {
                                let _ = edit_owner
                                    .update(cx, |this, cx| this.edit_label(id, window, cx));
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("label-delete-{id}")))
                            .ghost()
                            .small()
                            .label(t!("projects.common.delete"))
                            .disabled(busy)
                            .on_click(move |_, window, cx| {
                                let owner = delete_owner.clone();
                                confirm(
                                    window,
                                    cx,
                                    t!("projects.labels.delete_title", name = name),
                                    t!("projects.labels.delete_desc").into(),
                                    move |_, cx| {
                                        let _ =
                                            owner.update(cx, |this, cx| this.delete_label(id, cx));
                                    },
                                );
                            }),
                    ),
            ));
        }
        vec![toolbar, card(c, vec![form]), card(c, rows)]
    }

    // ---- custom fields ----

    pub(crate) fn save_field(&mut self, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        let f = &self.forms;
        let name = f.field_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.notice(t!("projects.common.name_required"));
            cx.notify();
            return;
        }
        let editing = f.editing_field;
        let field_type = editing
            .and_then(|id| self.fields.iter().find(|x| x.id == id))
            .map(|x| x.field_type)
            .unwrap_or(f.field_type);
        let existing = editing
            .and_then(|id| self.fields.iter().find(|x| x.id == id))
            .and_then(|x| x.options.clone());
        // select 以外に options を送ると backend が 400 を返す。
        let options = (field_type == CustomFieldType::Select)
            .then(|| parse_options(&f.field_options.read(cx).value(), existing.as_ref()));
        if options
            .as_ref()
            .is_some_and(|o| o.as_array().is_some_and(Vec::is_empty))
        {
            self.notice(t!("projects.fields.options_required"));
            cx.notify();
            return;
        }
        let required = f.field_required;
        let position = self.fields.iter().map(|x| x.position).max().unwrap_or(-1) + 1;
        self.call(
            cx,
            async move {
                match editing {
                    Some(id) => {
                        let req = UpdateCustomFieldRequest {
                            name: Some(name),
                            is_required: Some(required),
                            options,
                            position: None,
                        };
                        client.update_custom_field(tenant, project, id, &req).await
                    }
                    None => {
                        let req = CreateCustomFieldRequest {
                            name,
                            field_type,
                            is_required: Some(required),
                            options,
                            position: Some(position),
                        };
                        client.create_custom_field(tenant, project, &req).await
                    }
                }
            },
            |this, _, cx| this.after_catalog_change(Section::Fields, cx),
        );
    }

    fn edit_field(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(x) = self.fields.iter().find(|x| x.id == id).cloned() else {
            return;
        };
        let f = &mut self.forms;
        f.editing_field = Some(id);
        f.field_type = x.field_type;
        f.field_required = x.is_required;
        f.field_name
            .update(cx, |i, cx| i.set_value(x.name.clone(), window, cx));
        let options = serialize_options(x.options.as_ref());
        f.field_options
            .update(cx, |i, cx| i.set_value(options, window, cx));
        cx.notify();
    }

    fn delete_field(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        self.call(
            cx,
            async move { client.delete_custom_field(tenant, project, id).await },
            |this, (), cx| this.after_catalog_change(Section::Fields, cx),
        );
    }

    pub(crate) fn fields_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let f = &self.forms;
        let busy = self.busy > 0;
        let editing = f.editing_field.is_some();
        let current_type = f.field_type;
        let this = cx.entity().downgrade();
        let type_owner = this.clone();
        let type_button = Button::new("field-type")
            .small()
            .outline()
            .dropdown_caret(true)
            // 種類は作成後に変えられない（API に無い）。
            .disabled(editing)
            .label(field_type_label(current_type))
            .dropdown_menu(move |mut menu, _, _| {
                for ty in FIELD_TYPES {
                    let owner = type_owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(field_type_label(ty))
                            .checked(ty == current_type)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.forms.field_type = ty;
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            });
        let form = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(if editing {
                        t!("projects.fields.edit")
                    } else {
                        t!("projects.fields.new")
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(div().flex_1().child(field(
                        c,
                        t!("projects.common.name"),
                        Input::new(&f.field_name),
                    )))
                    .child(field(c, t!("projects.fields.type"), type_button)),
            )
            .when(current_type == CustomFieldType::Select, |d| {
                d.child(field(
                    c,
                    t!("projects.fields.options"),
                    Input::new(&f.field_options),
                ))
            })
            .child(
                Switch::new("field-required")
                    .checked(f.field_required)
                    .label(t!("projects.fields.required"))
                    .on_click(cx.listener(|this, v: &bool, _, cx| {
                        this.forms.field_required = *v;
                        cx.notify();
                    })),
            )
            .child(self.form_buttons(
                "field",
                editing,
                cx,
                |this, cx| this.save_field(cx),
                Section::Fields,
            ))
            .into_any_element();

        let mut rows = vec![];
        if self.is_loading(Section::Fields) {
            rows.push(placeholder(c, t!("projects.common.loading")));
        } else if self.fields.is_empty() {
            rows.push(placeholder(c, t!("projects.fields.empty")));
        }
        for x in &self.fields {
            let id = x.id;
            let name = x.name.clone();
            let (edit_owner, delete_owner) = (this.clone(), this.clone());
            let options = serialize_options(x.options.as_ref());
            rows.push(row(
                c,
                None,
                x.name.clone(),
                Some(options),
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(badge(c, field_type_label(x.field_type), None))
                    .when(x.is_required, |d| {
                        d.child(badge(c, t!("projects.fields.required"), Some(c.warning)))
                    })
                    .child(
                        Button::new(SharedString::from(format!("field-edit-{id}")))
                            .ghost()
                            .small()
                            .label(t!("projects.common.edit"))
                            .on_click(move |_, window, cx| {
                                let _ = edit_owner
                                    .update(cx, |this, cx| this.edit_field(id, window, cx));
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("field-delete-{id}")))
                            .ghost()
                            .small()
                            .label(t!("projects.common.delete"))
                            .disabled(busy)
                            .on_click(move |_, window, cx| {
                                let owner = delete_owner.clone();
                                confirm(
                                    window,
                                    cx,
                                    t!("projects.fields.delete_title", name = name),
                                    t!("projects.fields.delete_desc").into(),
                                    move |_, cx| {
                                        let _ =
                                            owner.update(cx, |this, cx| this.delete_field(id, cx));
                                    },
                                );
                            }),
                    ),
            ));
        }
        vec![card(c, vec![form]), card(c, rows)]
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_options, serialize_options};

    #[test]
    fn options_roundtrip_dedupes_and_keeps_labels() {
        let existing = serde_json::json!([{ "label": "High!", "value": "high" }]);
        let parsed = parse_options(" high, low ,, low\nmid", Some(&existing));
        assert_eq!(
            parsed,
            serde_json::json!([
                { "label": "High!", "value": "high" },
                { "label": "low", "value": "low" },
                { "label": "mid", "value": "mid" },
            ])
        );
        assert_eq!(serialize_options(Some(&parsed)), "high, low, mid");
        assert_eq!(serialize_options(None), "");
    }
}
