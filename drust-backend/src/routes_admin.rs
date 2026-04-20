// src/routes_admin.rs
use axum::{extract::{Path, State}, http::StatusCode, Json};
use serde::{Serialize, Deserialize};

use crate::state::AppState;
use crate::loader::{load_rules_from_dir, compile_rule, load_rules_from_gcs};
use crate::dto::RuleFile;
use crate::models::RuleRegistry;



#[derive(Serialize)]
pub struct ListItem {
    rule_id: String,
    versions: Vec<VersionItem>,
}
#[derive(Serialize)]
pub struct VersionItem {
    version: String,
    weight: u8,
}


/*pub async fn reload_rules(State(state): State<AppState>) -> StatusCode {
    let mut reg = state.write().await;
    match load_rules_from_dir("./rules", &mut reg) {
        Ok(n) => {
            tracing::info!("Reloaded {} rule file(s) from ./rules", n);
            StatusCode::OK
        }
        Err(e) => {
            tracing::error!("Reload failed: {:#}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}*/

pub async fn reload_rules(State(state): State<AppState>) -> (StatusCode, String) {
    let src = std::env::var("RULES_SRC").unwrap_or_else(|_| "./rules".to_string());

    // Load into a temp registry without blocking readers/writers
    let mut tmp = RuleRegistry::default();
    let res = if src.starts_with("gs://") {
        load_rules_from_gcs(&src, &mut tmp).await
    } else {
        load_rules_from_dir(&src, &mut tmp).map_err(anyhow::Error::from)
    };

    match res {
        Ok(n) => {
            // Swap in atomically
            {
                let mut reg = state.write().await;
                *reg = tmp;
            }
            tracing::info!("Reloaded {} rule file(s) from {}", n, src);
            (StatusCode::OK, format!("reloaded {n} from {src}"))
        }
        Err(e) => {
            tracing::error!("Reload failed from {}: {:#}", src, e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("reload failed: {e:#}"))
        }
    }
}

#[derive(Deserialize)]
pub struct UpsertRulePayload { pub yaml: String }

/*pub async fn upsert_rule(
    State(state): State<AppState>,
    Json(body): Json<UpsertRulePayload>,
) -> StatusCode {
    // Parse YAML -> RuleFile
    let parsed = match serde_yaml::from_str::<RuleFile>(&body.yaml) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("YAML parse error: {:#}", e);
            return StatusCode::BAD_REQUEST;
        }
    };

    // Compile RuleFile -> RuleEntry (IR)
    let entry = match compile_rule(parsed) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("compile_rule error: {:#}", e);
            return StatusCode::BAD_REQUEST;
        }
    };

    // Insert into registry
    let mut reg = state.write().await;
    reg.rules.insert(entry.id.clone(), entry);
    StatusCode::NO_CONTENT
}*/
pub async fn upsert_rule(
    State(state): State<AppState>,
    Json(body): Json<UpsertRulePayload>,
) -> (StatusCode, String) {
    let parsed = match serde_yaml::from_str::<RuleFile>(&body.yaml) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("YAML parse error: {e:#}")),
    };
    let entry = match compile_rule(parsed) {
        Ok(e) => e,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("compile_rule error: {e:#}")),
    };
    let mut reg = state.write().await;
    reg.rules.insert(entry.id.clone(), entry);
    (StatusCode::NO_CONTENT, String::new())
}



pub async fn list_rules(State(state): State<AppState>) -> Json<Vec<ListItem>> {
    let reg = state.read().await;
    let out = reg.rules.values().map(|entry| ListItem {
        rule_id: entry.id.clone(),
        versions: entry.versions.iter().map(|v| VersionItem {
            version: v.version.clone(),
            weight: v.weight,
        }).collect(),
    }).collect();
    Json(out)
}

pub async fn get_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<String>,
) -> Result<Json<ListItem>, StatusCode> {
    let reg = state.read().await;
    let entry = reg.rules.get(&rule_id).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(ListItem {
        rule_id: entry.id.clone(),
        versions: entry.versions.iter().map(|v| VersionItem {
            version: v.version.clone(),
            weight: v.weight,
        }).collect(),
    }))
}

pub async fn delete_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<String>,
) -> StatusCode {
    let mut reg = state.write().await;
    if reg.rules.remove(&rule_id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}