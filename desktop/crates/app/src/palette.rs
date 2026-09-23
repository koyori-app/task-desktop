//! Command and quick-search content hosted by gpui-kit's Dialog.
//! The kit owns the modal overlay, keyboard dismissal, and focus restoration.

use std::rc::Rc;

use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::label::Label;
use gpui_kit::component::{IndexPath, WindowExt};
use gpui_kit::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteKind {
    Commands,
    QuickSearch,
}

type QueryCallback = Rc<dyn Fn(&str, &mut Window, &mut App)>;
type ConfirmCallback = Rc<dyn Fn(IndexPath, &mut Window, &mut App)>;
type CancelCallback = Rc<dyn Fn(&mut Window, &mut App)>;

/// A separate entity keeps the dialog builder independent of AppShell's render
/// lease, while async search can replace the supplied results in place.
pub struct PaletteView {
    state: Entity<CommandState>,
    kind: PaletteKind,
    items: Vec<CommandItem>,
    loading: bool,
    footer: Option<SharedString>,
    on_query: QueryCallback,
    on_confirm: ConfirmCallback,
    on_cancel: CancelCallback,
}

impl PaletteView {
    /// Create a fresh view for each opening so query and selection start empty.
    pub fn new(
        kind: PaletteKind,
        items: Vec<CommandItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            state: cx.new(|cx| CommandState::new(window, cx)),
            kind,
            items,
            loading: false,
            footer: None,
            on_query: Rc::new(|_, _, _| {}),
            on_confirm: Rc::new(|_, _, _| {}),
            on_cancel: Rc::new(|_, _| {}),
        }
    }

    pub fn on_query(mut self, callback: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_query = Rc::new(callback);
        self
    }

    /// The dialog has already closed when this callback runs. Update parent
    /// bookkeeping and dispatch the selected action without closing it again.
    pub fn on_confirm(
        mut self,
        callback: impl Fn(IndexPath, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_confirm = Rc::new(callback);
        self
    }

    /// Called for the kit's Escape/backdrop dismissal. The kit performs the
    /// actual close; this callback only clears the parent palette/search state.
    pub fn on_cancel(mut self, callback: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Rc::new(callback);
        self
    }

    /// Replace async results without reopening the dialog or resetting its query.
    /// `footer` can carry a search error; None uses the standard keyboard hint.
    pub fn set_results(
        &mut self,
        items: Vec<CommandItem>,
        loading: bool,
        footer: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.items = items;
        self.loading = loading;
        self.footer = footer;
        cx.notify();
    }

    /// Host the view in the kit modal layer. The builder captures only the child
    /// entity and callbacks, never reads or updates its parent during rendering.
    pub fn open(view: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let state = view.read(cx).state.clone();
        let on_cancel = view.read(cx).on_cancel.clone();
        let content = view.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let on_cancel = on_cancel.clone();
            dialog
                .width(px(560.))
                .p_0()
                .close_button(false)
                .child(content.clone())
                .on_close(move |_, window, cx| on_cancel(window, cx))
        });
        // Root first captures the previous focus for its normal restoration.
        // Focus the Command input after its modal container has been installed.
        window.defer(cx, move |window, cx| {
            state.update(cx, |state, cx| state.focus(window, cx));
        });
    }
}

impl Render for PaletteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.read(cx).is_loading() != self.loading {
            self.state
                .update(cx, |state, cx| state.set_loading(self.loading, window, cx));
        }
        let placeholder = match self.kind {
            PaletteKind::Commands => "Type a command…",
            PaletteKind::QuickSearch => "Jump to a task or project…",
        };
        let footer = if self.loading {
            SharedString::from("Searching projects and tasks…")
        } else {
            self.footer
                .clone()
                .unwrap_or_else(|| "↑ ↓ to navigate · Enter to open · Esc to close".into())
        };
        let on_query = self.on_query.clone();
        let on_confirm = self.on_confirm.clone();
        Command::new(&self.state)
            .items(self.items.clone())
            .placeholder(placeholder)
            .bordered(false)
            .filterable(self.kind == PaletteKind::Commands)
            .max_h((window.viewport_size().height * 0.6).min(px(420.)))
            .w_full()
            .on_query(move |query, window, cx| on_query(query, window, cx))
            .on_confirm(move |path, window, cx| {
                window.close_dialog(cx);
                on_confirm(path, window, cx);
            })
            // Command clears a nonempty query on Escape, then lets an
            // empty-query Cancel propagate to the hosting Dialog for dismissal.
            .footer(move |_, _, cx| {
                Label::new(footer.clone())
                    .px_3()
                    .py_2()
                    .text_sm()
                    .text_color(crate::theme::colors(cx).text_muted)
            })
    }
}
