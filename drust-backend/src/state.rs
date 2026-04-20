// src/state.rs
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::models::RuleRegistry;

/// Shared application state used by routes
pub type AppState = Arc<RwLock<RuleRegistry>>;
