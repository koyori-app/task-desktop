//! desktop.md §16-18 Reviews。
//! PR 一覧（blocking/unresolved/rounds）、Detail で gate・Freshness・
//! Findings（available_actions は backend 駆動 §18.1）とレビュー作成。

mod detail;
mod list;

pub use detail::ReviewDetailView;
pub use list::{ReviewListEvent, ReviewListView};
