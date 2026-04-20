/*use axum::{extract::{Path, Query, State}, Json};
//use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{registry::RuleRegistry, executor, logger};
use crate::models::{ExecQuery, RuleResult};*/

use axum::{Json, extract::{Path, Query, State}, http::StatusCode};
use crate::models::{ExecQuery, RuleResult, RuleInput, RuleRegistry};
use crate::{executor, logger};
use crate::executor::{execute_version, execute_graph, log_file};
use std::sync::Arc;
use tokio::sync::RwLock;

use uuid::Uuid;
use std::time::Instant;


/*
#[derive(Deserialize)]
pub struct ExecQuery {
    version: Option<String>,
}*/

pub async fn execute_rule(
    Path(rule_id): Path<String>,
    Query(params): Query<ExecQuery>,
    State(registry): State<Arc<RwLock<RuleRegistry>>>,
    Json(body): Json<RuleInput>, // <-- now we receive input JSON
) -> Result<Json<RuleResult>, StatusCode> {
    let request_id = Uuid::new_v4().to_string();
    let start_time = Instant::now();

    let reg = registry.read().await;
    /*let selected = reg.select_version(&rule_id, params.version.as_deref());

    let Some(rule_version) = selected else { return Err(StatusCode::NOT_FOUND); };
    let decision = execute_version(&request_id, &rule_id, &rule_version, &body.data).await;*/
    let Some(rule_version) = reg.select_version(&rule_id, params.version.as_deref()) else {
        return Err(StatusCode::NOT_FOUND);
    };
    //let (decision, cond_results) = execute_version(&request_id, &rule_id, &rule_version, &body.data).await?;
    let (decision, cond_results) = if !rule_version.nodes.is_empty() {
        // Use the graph executor (START/CASE/ALL/ANY/RETURN)
        execute_graph(&request_id, &rule_id, &rule_version, &body.data).await?
    } else {
        // Fallback to the linear ALL/ANY executor
        execute_version(&request_id, &rule_id, &rule_version, &body.data).await?
    };
    let elapsed = start_time.elapsed().as_secs_f64();
    // final summary line in file
    log_file(
        r#"C:\Users\David\Documents\rust-projects\drust-backend-logs\log.txt"#,
        &format!(
            "[{}] {}:{} DECISION={} elapsed={:.3}s",
            request_id, rule_id, rule_version.version, decision, elapsed
        ),
    );
    Ok(Json(RuleResult {
        request_id,
        rule_id,
        version: rule_version.version,
        decision,
        elapsed_secs: elapsed,
        conditions: cond_results, // include per-condition timings
    }))

    /*match selected {
        Some(rule_version) => {
            let request_id = Uuid::new_v4().to_string();
            let decision = executor::execute(&rule_id, &rule_version.version).await;


            logger::log_decision(&request_id, &rule_id, &rule_version.version, &decision);

            Ok(Json(RuleResult {
                request_id,
                rule_id,
                version: rule_version.version,
                decision,
            }))
        },
        None => Err(StatusCode::NOT_FOUND),
    }*/
}
