use std::collections::HashMap;
use rand::Rng;

//use crate::models::{RuleRegistry, RuleEntry, RuleVersion, RuleCondition, DecisionLogic};
use crate::models::*; //{DecisionLogic, RuleCondition, RuleVersion};




impl RuleRegistry {
    pub fn new() -> Self {
        Self { rules: HashMap::new() }
    }

    /// Weighted selection or explicit version override
    pub fn select_version(&self, rule_id: &str, version_hint: Option<&str>) -> Option<RuleVersion> {
        let entry = self.rules.get(rule_id)?;
        if let Some(vs) = version_hint {
            return entry.versions.iter().find(|v| v.version == vs).cloned();
        }
        // weighted pick
        let total: u32 = entry.versions.iter().map(|v| v.weight as u32).sum();
        if total == 0 {
            return None;
        }
        //let r = fastrand::u32(0..total);
        let r= rand::thread_rng().gen_range(0..total);
        let mut acc = 0u32;
        for v in &entry.versions {
            acc += v.weight as u32;
            if r < acc {
                return Some(v.clone());
            }
        }
        entry.versions.first().cloned()
    }
}


/*
impl RuleRegistry {
    pub fn new() -> Self {
        let mut reg = RuleRegistry::default();
        let conditions = vec![
            RuleCondition {
                id: "age_check".into(),
                api: "http://localhost:9000/user_age?id={user_id}".into(),
                pass_if: "age >= 18".into(),
            },
            RuleCondition {
                id: "location_check".into(),
                api: "http://localhost:9000/user_location?id={user_id}".into(),
                pass_if: "country == \"MX\"".into(),
            },
            RuleCondition {
                id: "terms_check".into(),
                api: "http://localhost:9000/user_terms?id={user_id}".into(),
                pass_if: "accepted_at < \"2025-08-31\"".into(),
            },
        ];
        let nodes = vec![
            Node {
                id: "start".into(),
                kind: NodeType::Start { next: "gate_all".into() },
            },
            Node {
                id: "gate_all".into(),
                kind: NodeType::All {
                    conditions: conditions.clone(),
                    next_on_pass: "approve".into(),
                    next_on_fail: "reject".into(),
                },
            },
            Node {
                id: "approve".into(),
                kind: NodeType::Return { decision: "approved".into() },
            },
            Node {
                id: "reject".into(),
                kind: NodeType::Return { decision: "rejected".into() },
            },
        ];
        let v1 = RuleVersion {
            version: "v1".into(),
            weight: 100,
            conditions: conditions,
            logic: DecisionLogic::All,
            nodes, // <-- graph used by execute_graph
        };
        reg.rules.insert("credit_check".into(), RuleEntry {
            id: "credit_check".into(),
            versions: vec![v1],
        });

        reg
    }

    pub fn select_version(&self, rule_id: &str, user_version: Option<&str>) -> Option<RuleVersion> {
        self.rules.get(rule_id).map(|entry| {
            if let Some(v) = user_version {
                entry.versions.iter().find(|ver| ver.version == v).cloned().unwrap_or_else(|| entry.versions[0].clone())
            } else {
                // Weighted random selection
                let roll = rand::thread_rng().gen_range(0..100);
                let mut cumulative = 0;
                for v in &entry.versions {
                    cumulative += v.weight;
                    if roll < cumulative {
                        return v.clone();
                    }
                }
                entry.versions[0].clone()
            }
        })
    }
}
*/
