//! Application state, notification sync, settings, credentials.

pub mod auth;
mod error;
pub mod notify_text;
pub mod settings;
pub mod sync;

pub use error::{Error, Result};
pub use settings::{Appearance, NotificationPrefs, Settings, SettingsStore};
pub use sync::{NotificationEngine, TickOutcome};
