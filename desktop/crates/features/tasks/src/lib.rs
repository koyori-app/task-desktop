//! desktop.md §15 Tasks。
//! My Tasks / Project の一覧と、Detail ペインの
//! 表示・編集（Status/Priority/Assignee/DueDate/done）・コメント。

mod avatar;
mod detail;
mod list;
mod model;
mod ui;

pub use detail::{TaskDetailEvent, TaskDetailView};
pub use list::{ListMode, TaskListEvent, TaskListView};
pub use model::TaskRow;
