// dto.rs (new file)
use serde::Deserialize;

#[derive(Deserialize)]
pub struct RuleFile {
    pub rule_id: String,
    pub versions: Vec<RuleVersionFile>,
}

#[derive(Deserialize)]
pub struct RuleVersionFile {
    pub version: String,
    pub weight: u8,
    pub nodes: Vec<NodeFile>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum NodeFile {
    #[serde(rename = "START")]
    Start { id: String, next: String },

    #[serde(rename = "ALL")]
    All {
        id: String,
        conditions: Vec<ConditionFile>,
        next_on_pass: String,
        next_on_fail: String,
    },

    #[serde(rename = "ANY")]
    Any {
        id: String,
        conditions: Vec<ConditionFile>,
        next_on_pass: String,
        next_on_fail: String,
    },

    #[serde(rename = "CASE")]
    Case {
        id: String,
        branches: Vec<CaseBranchFile>,
        default: String,
    },

    #[serde(rename = "RETURN")]
    Return { id: String, decision: String },
    // You can add ACTION / CALL later the same way

    #[serde(rename = "ACTION")]
    Action {
        id: String,
        action: String,
        method: String,              // "GET" | "POST" (MVP)
        url: String,
        #[serde(default)]
        body: Option<String>,        // template, e.g. '{"msisdn":"{msisdn}"}'
        #[serde(default)]
        extract: Vec<ExtractFile>,   // same ExtractFile as conditions
        next: String,
    },
}

#[derive(Deserialize)]
pub struct ExtractFile {
    pub to: String,
    pub jsonpath: String,
}

#[derive(Deserialize)]
pub struct ConditionFile {
    pub id: String,
    pub api: String,
    pub pass_if: String,
    #[serde(default)]
    pub extract: Vec<ExtractFile>,
}

#[derive(Deserialize)]
pub struct CaseBranchFile {
    pub when: String,
    pub next: String,
}
