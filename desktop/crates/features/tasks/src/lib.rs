//! desktop.md §15 Tasks。
//! My Tasks / Today / Upcoming / Project の一覧と、Detail ペインの
//! 表示・編集（Status/Priority/Assignee/DueDate/done）・コメント。

mod detail;
mod list;
mod model;

pub use detail::TaskDetailView;
pub use list::{ListMode, TaskListEvent, TaskListView};
pub use model::TaskRow;
