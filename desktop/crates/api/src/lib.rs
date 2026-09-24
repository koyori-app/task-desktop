//! Koyori API client: OpenAPI generated types + reqwest.

mod avatar;
mod client;
mod error;
pub mod spec;
pub mod types {
    #![allow(dead_code)]
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/types.rs"));
}

pub use avatar::{AvatarBytes, fetch_avatar};
pub use client::{Client, FindingsQuery, MyTasksQuery, NotificationsQuery, TasksQuery};
pub use error::{ApiError, Result};
