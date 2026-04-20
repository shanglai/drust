
mod models;
mod dto;
mod registry;
mod state;
mod loader;
mod executor;
mod routes;
mod routes_admin;
mod logger;

use std::env;
use anyhow::Result;
use axum::{Router, routing::{get, post}, http::StatusCode};
use tokio::net::TcpListener;

use crate::{state::AppState, models::RuleRegistry};
//use crate::routes;
//use crate::routes_admin;
use crate::loader::{load_rules_from_dir,load_rules_from_gcs};

#[tokio::main]
async fn main() -> Result<()> {
    // 1) show panics in logs
    std::panic::set_hook(Box::new(|info| {
        eprintln!("PANIC: {info}");
    }));
    // logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::INFO) // or EnvFilter if you enabled the feature
        .init();

    // shared state
    let state: AppState = std::sync::Arc::new(tokio::sync::RwLock::new(RuleRegistry::default()));

    // build router (add healthz)
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/execute/:rule_id", post(routes::execute_rule))
        .route("/admin/rules", get(routes_admin::list_rules))
        .route("/admin/rules/:rule_id", get(routes_admin::get_rule))
        .route("/admin/rules/:rule_id", axum::routing::delete(routes_admin::delete_rule))
        .route("/admin/rules/reload", post(routes_admin::reload_rules))
        .route("/admin/rules/upsert", post(routes_admin::upsert_rule))
        .with_state(state.clone());
    // 2) after building the router, before binding:
    tracing::info!("Building router done");
    // bind ASAP (Cloud Run requires listening quickly)
    let port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    // 3) right before bind:
    tracing::info!("Binding to 0.0.0.0:{}", port);
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Building router done");
    tracing::info!("Drust-core listening on {addr}");
    // 4) right after bind, before serve:
    tracing::info!("Listener bound, starting axum::serve");

    // kick off background rules load (don’t block readiness)
    let rules_src = env::var("RULES_SRC").unwrap_or_else(|_| "/app/rules".to_string());
    let state_for_loader = state.clone();
    tokio::spawn(async move {
        if let Err(e) = background_load(&rules_src, state_for_loader).await {
            tracing::error!("Startup rules load failed from {}: {:#}", rules_src, e);
        } else {
            tracing::info!("Startup rules load OK from {}", rules_src);
        }
    });

    // IMPORTANT: await server forever (do NOT spawn and return)
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn background_load(src: &str, state: AppState) -> Result<()> {
    let mut tmp = RuleRegistry::default();
    if src.starts_with("gs://") {
        load_rules_from_gcs(src, &mut tmp).await?;
    } else {
        load_rules_from_dir(src, &mut tmp)?;
    }
    let mut reg = state.write().await;
    *reg = tmp;
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal;
    let _ = signal::ctrl_c().await;
    tracing::info!("Shutdown signal received");
}
