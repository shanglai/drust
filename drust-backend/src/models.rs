use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use serde_json::Value;

/// Query parameters for executing a rule
#[derive(Debug, Deserialize)]
pub struct ExecQuery {
    pub version: Option<String>,
}

/// Input data structure for rule execution
#[derive(Debug, Deserialize)]
pub struct RuleInput {
    // You can define structured input here later
    // For now, we use untyped input
    pub data: serde_json::Value,
}

/// Output of the rule execution
#[derive(Debug, Serialize)]
pub struct RuleResult {
    pub request_id: String,
    pub rule_id: String,
    pub version: String,
    pub decision: String,
    pub elapsed_secs: f64, // seconds.milliseconds
    pub conditions: Vec<ConditionResult> // NEW
}


#[derive(Debug, Clone)]
pub enum DecisionLogic {
    All,
    Any,
}

#[derive(Debug, Clone)]
pub struct ExtractSpec {
    pub to: String,       // variable name in ctx.vars (e.g., "imsi")
    pub jsonpath: String, // JSONPath in the API response (e.g., "$.imsi" or "$.data[0].id")
}

#[derive(Debug, Clone)]
pub struct RuleCondition {
    pub id: String,
    pub api: String,
    pub pass_if: String, // Can parse as expression or simple enum
    pub extract: Vec<ExtractSpec>, // NEW (can be empty)
}

#[derive(Debug, Clone)]
pub struct RuleVersion {
    pub version: String,
    pub weight: u8, // 0-100
    pub conditions: Vec<RuleCondition>,
    pub logic: DecisionLogic, // e.g. All, Any
    pub nodes: Vec<Node>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ConditionResult {
    pub id: String,
    pub passed: bool,
    pub elapsed_ms: u128, // total per-condition time
}

#[derive(Debug, Clone)]
pub struct RuleEntry {
    pub id: String,
    pub versions: Vec<RuleVersion>,
}

#[derive(Debug, Default)]
pub struct RuleRegistry {
    pub rules: HashMap<String, RuleEntry>,
}



//Data Model
#[derive(Debug, Clone)]
pub enum NodeType {
    Start { next: String },
    All { conditions: Vec<RuleCondition>, next_on_pass: String, next_on_fail: String },
    Any { conditions: Vec<RuleCondition>, next_on_pass: String, next_on_fail: String },
    Case { branches: Vec<CaseBranch>, default: String },
    Call { target_rule: String, target_version: Option<String>, bind: serde_json::Value, result_map: Option<serde_json::Value>, next: String },
    //Action { action: String, next: String },
    Return { decision: String },
    Action {
        action: String,
        method: String,
        url: String,
        body: Option<String>,
        extract: Vec<ExtractSpec>,   // same ExtractSpec you added for conditions
        next: String,
    },
}

#[derive(Debug, Clone)]
pub struct CaseBranch {
    pub when: String,  // expression, same evaluator as pass_if
    pub next: String,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub kind: NodeType,
}



#[derive(Clone, Debug)]
pub struct ExecutionContext {
    pub vars: HashMap<String, Value>,
    pub cache: HashMap<String, Value>, // cond.id -> raw response
}