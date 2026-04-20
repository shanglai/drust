// src/loader.rs
use std::{fs, path::{Path, PathBuf}};
use anyhow::{Context, Result};
use std::time::Duration;
use reqwest::Client;
use serde::Deserialize;

use crate::dto::*;
use crate::models::*;
//use crate::registry::RuleRegistry;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
    token_type: Option<String>,
}

pub fn load_rules_from_dir<P: AsRef<Path>>(dir: P, registry: &mut RuleRegistry) -> Result<usize> {
    let dir = dir.as_ref();
    if !dir.exists() {
        fs::create_dir_all(dir).with_context(|| format!("create rules dir {:?}", dir))?;
        return Ok(0);
    }

    let mut loaded = 0usize;

    for entry in fs::read_dir(dir).with_context(|| format!("read_dir {:?}", dir))? {
        let entry = entry?;
        let path = entry.path();
        if !is_yaml(&path) { continue; }

        let text = fs::read_to_string(&path)
            .with_context(|| format!("read_to_string {:?}", path))?;

        match serde_yaml::from_str::<RuleFile>(&text) {
            Ok(parsed) => match compile_rule(parsed) {
                Ok(entry) => {
                    registry.rules.insert(entry.id.clone(), entry);
                    loaded += 1;
                }
                Err(e) => eprintln!("compile_rule failed for {:?}: {:#}", path, e),
            },
            Err(e) => eprintln!("serde_yaml parse failed for {:?}: {:#}", path, e),
        }
    }

    Ok(loaded)
}

//GCP and Rules
pub async fn gcp_token() -> Result<String, anyhow::Error> {
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    // 1️⃣ Try metadata server (Cloud Run / GKE)
    let meta_url = "http://metadata/computeMetadata/v1/instance/service-accounts/default/token";
    let metadata_token = client
        .get(meta_url)
        .header("Metadata-Flavor", "Google")
        .send()
        .await;

    if let Ok(resp) = metadata_token {
        if resp.status().is_success() {
            let token_data: TokenResponse = resp.json().await?;
            return Ok(token_data.access_token);
        }
    }

    // 2️⃣ Fall back to Application Default Credentials (local dev / service account key)
    match gcp_auth::AuthenticationManager::new().await {
        Ok(manager) => {
            let token = manager
                .get_token(&["https://www.googleapis.com/auth/cloud-platform"])
                .await?;
            Ok(token.as_str().to_string())
        }
        Err(e) => Err(anyhow::anyhow!("Failed to get GCP token: {}", e)),
    }
}

pub async fn load_rules_from_gcs(uri: &str, registry: &mut RuleRegistry) -> anyhow::Result<usize> {
    // parse gs://bucket/prefix
    let (bucket, prefix) = uri.strip_prefix("gs://")
        .and_then(|s| s.split_once('/'))
        .ok_or_else(|| anyhow::anyhow!("Invalid RULES_SRC: {uri}"))?;
    // list objects
    //let token = gcp_token("https://www.googleapis.com/auth/devstorage.read_only").await?;
    let token= gcp_token().await?;
    let client = reqwest::Client::new();
    let mut loaded = 0usize;
    let mut page_token: Option<String> = None;

    loop {
        let mut url = format!(
            "https://storage.googleapis.com/storage/v1/b/{}/o?prefix={}",
            urlencoding::encode(bucket),
            urlencoding::encode(prefix)
        );
        if let Some(pt) = &page_token {
            url.push_str("&pageToken=");
            url.push_str(&urlencoding::encode(pt));
        }
        let list = client.get(&url)
            .bearer_auth(&token)
            .send().await?
            .error_for_status()?
            .json::<serde_json::Value>().await?;

        if let Some(items) = list["items"].as_array() {
            for it in items {
                let name = it["name"].as_str().unwrap_or_default();
                // filter yml/yaml
                if !(name.ends_with(".yml") || name.ends_with(".yaml")) { continue; }

                // download
                let media_url = format!(
                    "https://storage.googleapis.com/storage/v1/b/{}/o/{}?alt=media",
                    urlencoding::encode(bucket),
                    urlencoding::encode(name),
                );
                let text = client.get(&media_url)
                    .bearer_auth(&token)
                    .send().await?
                    .error_for_status()?
                    .text().await?;

                // parse + compile (reusing your loader/compile)
                match serde_yaml::from_str::<RuleFile>(&text)
                    .map_err(anyhow::Error::from)          // unify serde_yaml::Error -> anyhow::Error
                    .and_then(|rf| crate::loader::compile_rule(rf)) // anyhow::Result<RuleEntry>
                {
                    Ok(entry) => {
                        registry.rules.insert(entry.id.clone(), entry);
                        loaded += 1;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load rule from {}: {:#}", name, e);
                        // continue;
                    }
                }

            }
        }
        page_token = list["nextPageToken"].as_str().map(|s| s.to_string());
        if page_token.is_none() { break; }
    }

    Ok(loaded)
}

fn is_yaml(path: &PathBuf) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()),
             Some(ref ext) if ext == "yml" || ext == "yaml")
}

// --- compile YAML DTOs into internal IR ---

pub fn compile_rule(rule: RuleFile) -> Result<RuleEntry> {
    if rule.versions.is_empty() { anyhow::bail!("no versions"); }

    let versions = rule.versions.into_iter().map(|v| {
        let nodes = v.nodes.into_iter().map(|n| {
            Ok(match n {
                NodeFile::Start { id, next } => Node { id, kind: NodeType::Start { next } },
                NodeFile::All { id, conditions, next_on_pass, next_on_fail } => Node {
                    id,
                    kind: NodeType::All {
                        conditions: conditions.into_iter().map(|c| RuleCondition {
                            id: c.id, api: c.api, pass_if: c.pass_if,
                            extract: c.extract.into_iter().map(|e| ExtractSpec { to: e.to, jsonpath: e.jsonpath }).collect(),
                        }).collect(),
                        next_on_pass, next_on_fail,
                    },
                },
                NodeFile::Any { id, conditions, next_on_pass, next_on_fail } => Node {
                    id,
                    kind: NodeType::Any {
                        conditions: conditions.into_iter().map(|c| RuleCondition {
                            id: c.id, api: c.api, pass_if: c.pass_if,
                            extract: c.extract.into_iter().map(|e| ExtractSpec { to: e.to, jsonpath: e.jsonpath }).collect(),
                        }).collect(),
                        next_on_pass, next_on_fail,
                    },
                },
                NodeFile::Case { id, branches, default } => Node {
                    id,
                    kind: NodeType::Case {
                        branches: branches.into_iter().map(|b| CaseBranch { when: b.when, next: b.next }).collect(),
                        default,
                    },
                },
                NodeFile::Return { id, decision } => Node { id, kind: NodeType::Return { decision } },
                NodeFile::Action { id, action, method, url, body, extract, next } => Node {
                    id,
                    kind: NodeType::Action {
                        action,
                        method,
                        url,
                        body,
                        extract: extract.into_iter().map(|e| ExtractSpec { to: e.to, jsonpath: e.jsonpath }).collect(),
                        next,
                    },
                },
            })
        }).collect::<Result<Vec<_>>>()?;

        ensure_has_start(&nodes)?;
        ensure_targets_exist(&nodes)?;

        Ok(RuleVersion {
            version: v.version,
            weight: v.weight,
            // legacy linear fields still present if you use them elsewhere
            conditions: vec![],
            logic: DecisionLogic::All,
            nodes,
        })
    }).collect::<Result<Vec<_>>>()?;

    Ok(RuleEntry { id: rule.rule_id, versions })
}

fn ensure_has_start(nodes: &[Node]) -> Result<()> {
    if nodes.iter().any(|n| matches!(n.kind, NodeType::Start { .. })) { Ok(()) }
    else { anyhow::bail!("missing START node") }
}


fn ensure_targets_exist(nodes: &[Node]) -> Result<()> {
    use anyhow::bail;
    use std::collections::HashSet;

    let ids: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();

    for n in nodes {
        match &n.kind {
            NodeType::Start { next } => {
                if !ids.contains(next.as_str()) {
                    bail!("bad edge: START -> {} (target not found)", next);
                }
            }

            NodeType::All { next_on_pass, next_on_fail, .. }
            | NodeType::Any { next_on_pass, next_on_fail, .. } => {
                if !ids.contains(next_on_pass.as_str()) {
                    bail!("bad edge: {} -> {} (next_on_pass not found)", n.id, next_on_pass);
                }
                if !ids.contains(next_on_fail.as_str()) {
                    bail!("bad edge: {} -> {} (next_on_fail not found)", n.id, next_on_fail);
                }
            }

            NodeType::Case { branches, default } => {
                for b in branches {
                    if !ids.contains(b.next.as_str()) {
                        bail!("bad edge: {} branch -> {} (target not found)", n.id, b.next);
                    }
                }
                if !ids.contains(default.as_str()) {
                    bail!("bad edge: {} default -> {} (target not found)", n.id, default);
                }
            }

            NodeType::Action { next, .. } => {
                // ACTION must point somewhere valid
                if !ids.contains(next.as_str()) {
                    bail!("bad edge: ACTION {} -> {} (target not found)", n.id, next);
                }
            }

            NodeType::Call { target_rule, next, .. } => {
                // CALL must name a non-empty target rule and a valid next
                if target_rule.trim().is_empty() {
                    bail!("bad call: node {} has empty target_rule", n.id);
                }
                if !ids.contains(next.as_str()) {
                    bail!("bad edge: CALL {} -> {} (next not found)", n.id, next);
                }
                // Optional: you could also validate `target_version` format if Some(...)
                // and even lint `bind` / `result_map` shapes if you define a schema.
            }

            NodeType::Return { .. } => {
                // terminal node: no edges to validate
            }
        }
    }

    Ok(())
}
