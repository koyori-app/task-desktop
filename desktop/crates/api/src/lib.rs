//! Koyori API client: OpenAPI generated types + reqwest.

mod client;
mod error;
pub mod spec;
pub mod types {
    #![allow(dead_code)]
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/types.rs"));
}

pub use client::{Client, FindingsQuery, MyTasksQuery, NotificationsQuery, TasksQuery};
pub use error::{ApiError, Result};
