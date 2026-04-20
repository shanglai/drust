use tracing_subscriber;

pub fn init() {
    tracing_subscriber::fmt().with_target(false).init();
}

pub fn log_decision(request_id: &str, rule_id: &str, version: &str, decision: &str) {
    tracing::info!(request_id, rule_id, version, decision, "Rule executed");
}
