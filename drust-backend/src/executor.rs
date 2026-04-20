
use chrono::{NaiveDate, Utc, Duration};
use chrono_tz::Tz;
use serde_json::Value;
use reqwest::Client;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::Instant;
use std::collections::HashMap;
use axum::http::StatusCode;
use crate::models::{RuleVersion, RuleCondition, DecisionLogic, ConditionResult, Node, NodeType, ExecutionContext, ExtractSpec, RuleRegistry};
use log::error;

const LOG_PATH: &str = r#"C:\Users\David\Documents\rust-projects\drust-backend-logs\log.txt"#;


impl ExecutionContext {
    pub fn new(initial: &Value) -> Self {
        // seed with input payload fields as top-level vars, if you want:
        let mut vars = HashMap::new();
        if let Some(obj) = initial.as_object() {
            for (k, v) in obj {
                vars.insert(k.clone(), v.clone());
            }
        }
        Self { vars, cache: HashMap::new() }
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        self.vars.get(key).and_then(|v| v.as_str().map(|s| s.to_string()))
    }

    pub fn set_var(&mut self, key: impl Into<String>, val: Value) {
        self.vars.insert(key.into(), val);
    }

    /// Build a temporary JSON object view of all current variables.
    /// (Vars already include initial input fields from `ExecutionContext::new(input)`.)
    pub fn vars_json(&self) -> serde_json::Value {
        use serde_json::{Map, Value};
        let mut m = Map::with_capacity(self.vars.len());
        for (k, v) in &self.vars {
            m.insert(k.clone(), v.clone());
        }
        Value::Object(m)
    }
}


/*/// Replace placeholders like {user_id} in URLs with values from `input` JSON.
fn interpolate(url: &str, input: &Value) -> String {
    // very small interpolator for {user_id}
    let mut out = url.to_string();
    if let Some(user_id) = input.get("user_id").and_then(|v| v.as_str()) {
        out = out.replace("{user_id}", user_id);
    }
    out
}*/
fn interpolate(url: &str, ctx: &ExecutionContext) -> String {
    // Replace occurrences of {var} with ctx.vars[var].as_str() or as JSON string fallback
    let mut out = String::with_capacity(url.len());
    let mut chars = url.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            // read until '}'
            let mut key = String::new();
            while let Some(&nc) = chars.peek() {
                chars.next();
                if nc == '}' { break; }
                key.push(nc);
            }
            if let Some(v) = ctx.vars.get(&key) {
                if let Some(s) = v.as_str() { out.push_str(s); }
                else { out.push_str(&v.to_string()); }
            } else {
                // keep placeholder as-is if missing
                out.push('{'); out.push_str(&key); out.push('}');
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn apply_extracts(ctx: &mut ExecutionContext, cond_id: &str, resp: &Value, extracts: &[ExtractSpec]) {
    // cache full response
    ctx.cache.insert(cond_id.to_string(), resp.clone());

    for ex in extracts {
        if let Ok(found) = jsonpath_lib::select(resp, &ex.jsonpath) {
            // take first match; you can extend to lists if you want
            if let Some(first) = found.first() {
                ctx.set_var(&ex.to, (*first).clone());
            }
        }
    }
}


// return decision + per-condition timings
pub async fn execute_version(
    request_id: &str,
    rule_id: &str,
    version: &RuleVersion,
    input: &Value,
) -> Result<(String, Vec<ConditionResult>), StatusCode> {
    let client = Client::new();
    let all = matches!(version.logic, DecisionLogic::All);
    let mut passed_any = false;
    let mut cond_results: Vec<ConditionResult> = Vec::new();

    // NEW: per-request execution context (vars + cache)
    let mut ctx = ExecutionContext::new(input);

    for cond in &version.conditions {
        // CHANGED: interpolate using ctx.vars (supports {imsi}, etc.)
        let url = interpolate(&cond.api, &ctx);

        let cond_start = Instant::now();

        let http_start = Instant::now();
        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                log_file(r#"C:\Users\David\Documents\rust-projects\drust-backend-logs\log.txt"#, &format!(
                    "[{}] {}:{} cond={} HTTP_ERROR {}",
                    request_id, rule_id, version.version, cond.id, e
                ));
                let elapsed_ms = cond_start.elapsed().as_millis();
                cond_results.push(ConditionResult { id: cond.id.clone(), passed: false, elapsed_ms });
                if all { return Ok(("rejected".into(), cond_results)); } else { continue; }
            }
        };
        let http_ms = http_start.elapsed().as_millis();

        let parse_start = Instant::now();
        let resp_json = match resp.json::<Value>().await {
            Ok(j) => j,
            Err(e) => {
                log_file(r#"C:\Users\David\Documents\rust-projects\drust-backend-logs\log.txt"#, &format!(
                    "[{}] {}:{} cond={} JSON_ERROR {}",
                    request_id, rule_id, version.version, cond.id, e
                ));
                let elapsed_ms = cond_start.elapsed().as_millis();
                cond_results.push(ConditionResult { id: cond.id.clone(), passed: false, elapsed_ms });
                if all { return Ok(("rejected".into(), cond_results)); } else { continue; }
            }
        };
        let parse_ms = parse_start.elapsed().as_millis();

        // NEW: cache response and extract variables into ctx.vars (e.g., set "imsi")
        apply_extracts(&mut ctx, &cond.id, &resp_json, &cond.extract);

        let eval_start = Instant::now();
        let ok = evaluate_condition_with_ctx(&ctx, &resp_json, &cond.pass_if);
        let eval_ms = eval_start.elapsed().as_millis();

        let total_ms = cond_start.elapsed().as_millis();

        log_file(
            r#"C:\Users\David\Documents\rust-projects\drust-backend-logs\log.txt"#,
            &format!(
                "[{}] {}:{} cond={} pass_if='{}' result={} http_ms={} parse_ms={} eval_ms={} total_ms={} vars_now={:?}",
                request_id, rule_id, version.version, cond.id, cond.pass_if, ok, http_ms, parse_ms, eval_ms, total_ms, ctx.vars
            ),
        );

        cond_results.push(ConditionResult {
            id: cond.id.clone(),
            passed: ok,
            elapsed_ms: total_ms,
        });

        if all && !ok {
            return Ok(("rejected".into(), cond_results));
        }
        if !all && ok {
            passed_any = true;
        }
    }

    let decision = if all { "approved" } else if passed_any { "approved" } else { "rejected" };
    Ok((decision.to_string(), cond_results))
}

/// Append a timestamped line to a log file (Windows path supported)
pub fn log_file(path: &str, msg: &str) {
    let timestamp = Utc::now().to_rfc3339();
    match OpenOptions::new().append(true).create(true).open(path) {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, "[{}] {}", timestamp, msg) {
                eprintln!("Failed to write log to {}: {}", path, e);
            }
        }
        Err(e) => {
            eprintln!("Failed to open log file {}: {}", path, e);
        }
    }
}

/// Evaluate a single condition against an API JSON `response`.
/// Supports:
///   - `<field> <op> <value>`  (==, !=, >, <, >=, <=)
///   - `<field> between <low> and <high>`  (inclusive)
pub fn evaluate_condition(response: &serde_json::Value, pass_if: &str) -> bool {
    let tokens = tokenize(pass_if);
    if tokens.is_empty() {
        return false;
    }

    // Pattern 1: <field> between <low> and <high>
    if tokens.len() >= 5
        && tokens[1].eq_ignore_ascii_case("between")
        && tokens[3].eq_ignore_ascii_case("and")
    {
        let field = tokens[0].as_str();
        let low_raw = tokens[2].as_str();
        let high_raw = tokens[4].as_str();

        let actual = &response[field];
        return compare_between(actual, low_raw, high_raw);
    }

    // Pattern 2: <field> <op> <value>
    if tokens.len() == 3 {
        let field = tokens[0].as_str();
        let op = tokens[1].as_str();
        let expected_raw = tokens[2].as_str();

        let actual = &response[field];
        return compare_op(actual, op, expected_raw);
    }

    false
}

/// Context-aware condition evaluation:
/// - tries response[field] first
/// - if null/missing, falls back to ctx.vars[field]
pub fn evaluate_condition_with_ctx(
    ctx: &ExecutionContext,
    response: &serde_json::Value,
    pass_if: &str,
) -> bool {
    let tokens = tokenize(pass_if);
    if tokens.is_empty() { return false; }

    // helper to resolve actual value
    let mut resolve_actual = |field: &str| -> &serde_json::Value {
        let v = &response[field];
        if !v.is_null() {
            v
        } else {
            ctx.vars.get(field).unwrap_or(&serde_json::Value::Null)
        }
    };

    // <field> between <low> and <high>
    if tokens.len() >= 5
        && tokens[1].eq_ignore_ascii_case("between")
        && tokens[3].eq_ignore_ascii_case("and")
    {
        let field = tokens[0].as_str();
        let low   = tokens[2].as_str();
        let high  = tokens[4].as_str();
        let actual = resolve_actual(field);
        return compare_between(actual, low, high);
    }

    // <field> <op> <value>
    if tokens.len() == 3 {
        let field        = tokens[0].as_str();
        let op           = tokens[1].as_str();
        let expected_raw = tokens[2].as_str();
        let actual = resolve_actual(field);
        return compare_op(actual, op, expected_raw);
    }

    false
}

// quoted-aware tokenizer: splits on whitespace outside quotes, strips the quotes
fn tokenize(expr: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buf = String::new();
    let mut in_quotes = false;

    for ch in expr.chars() {
        match ch {
            '"' => { in_quotes = !in_quotes; } // toggle, quotes not included
            c if c.is_whitespace() && !in_quotes => {
                if !buf.is_empty() {
                    tokens.push(std::mem::take(&mut buf));
                }
            }
            _ => buf.push(ch),
        }
    }
    if !buf.is_empty() {
        tokens.push(buf);
    }
    tokens
}

// choose a timezone for “today”. You can make this a config.
const TZ: Tz = chrono_tz::America::Mexico_City;

fn today_naive() -> NaiveDate {
    Utc::now().with_timezone(&TZ).date_naive()
}

/// Try to parse a raw literal into Number / Date / String, with support for:
/// - today()
/// - today()-Nd / today()+Nd  (e.g., today()-7d, today()+30d)
fn parse_expected(raw: &str) -> Expected {
    let unquoted = raw.trim_matches('"').trim();

    // today()
    if unquoted.eq_ignore_ascii_case("today()") {
        return Expected::Date(today_naive());
    }

    // today() +/- Nd
    // accepted forms: today()-7d, today() + 14d (spaces optional), case-insensitive 'd'
    if let Some(rest) = unquoted.strip_prefix("today()") {
        let rest = rest.trim();
        // e.g., "-7d" or "+30d"
        if let Some(sign) = rest.chars().next() {
            if sign == '+' || sign == '-' {
                let amt_str = rest[1..].trim();
                // allow optional spaces and trailing 'd'
                let amt_str = amt_str.trim_end_matches(|c: char| c.is_whitespace());
                let amt_str = amt_str.strip_suffix('d').unwrap_or(amt_str);
                if let Ok(days) = amt_str.trim().parse::<i64>() {
                    let base = today_naive();
                    let shifted = if sign == '+' {
                        base + Duration::days(days)
                    } else {
                        base - Duration::days(days)
                    };
                    return Expected::Date(shifted);
                }
            }
        }
    }

    // YYYY-MM-DD
    if let Ok(d) = NaiveDate::parse_from_str(unquoted, "%Y-%m-%d") {
        return Expected::Date(d);
    }
    // number
    if let Ok(n) = unquoted.parse::<f64>() {
        return Expected::Num(n);
    }
    // fallback string
    Expected::Str(unquoted.to_string())
}


enum Expected {
    Num(f64),
    Date(NaiveDate),
    Str(String),
}

fn compare_between(actual: &Value, low_raw: &str, high_raw: &str) -> bool {
    let low = parse_expected(low_raw);
    let high = parse_expected(high_raw);

    match actual {
        Value::Number(n) => {
            let a = n.as_f64().unwrap_or(f64::NAN);
            if let (Expected::Num(l), Expected::Num(h)) = (&low, &high) {
                return a >= *l && a <= *h;
            }
            false
        }

        Value::String(s) => {
            // 1) Date comparison
            if let Ok(a_date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                if let (Expected::Date(l), Expected::Date(h)) = (&low, &high) {
                    return a_date >= *l && a_date <= *h;
                }
                return false;
            }

            // 2) Numeric-as-string if bounds are numeric
            if let (Expected::Num(l), Expected::Num(h)) = (&low, &high) {
                if let Ok(a_num) = s.trim().parse::<f64>() {
                    return a_num >= *l && a_num <= *h;
                }
                return false;
            }

            // 3) Lexicographic strings
            if let (Expected::Str(l), Expected::Str(h)) = (&low, &high) {
                return s >= l && s <= h;
            }
            false
        }

        _ => false,
    }
}


fn compare_op(actual: &serde_json::Value, op: &str, expected_raw: &str) -> bool {
    let expected = parse_expected(expected_raw);

    match actual {
        serde_json::Value::Number(n) => {
            let a = n.as_f64().unwrap_or(f64::NAN);
            if let Expected::Num(e) = expected {
                match op {
                    "==" => a == e,
                    "!=" => a != e,
                    ">"  => a > e,
                    "<"  => a < e,
                    ">=" => a >= e,
                    "<=" => a <= e,
                    _ => false,
                }
            } else { false }
        }

        serde_json::Value::String(s) => {
            // dates first
            if let Ok(a_date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                if let Expected::Date(e_date) = expected {
                    return match op {
                        "==" => a_date == e_date,
                        "!=" => a_date != e_date,
                        ">"  => a_date >  e_date,
                        "<"  => a_date <  e_date,
                        ">=" => a_date >= e_date,
                        "<=" => a_date <= e_date,
                        _ => false,
                    };
                }
                return false;
            }
            // plain strings
            if let Expected::Str(e_str) = expected {
                match op {
                    "==" => s == &e_str,
                    "!=" => s != &e_str,
                    ">"  => s >  &e_str,
                    "<"  => s <  &e_str,
                    ">=" => s >= &e_str,
                    "<=" => s <= &e_str,
                    _ => false,
                }
            } else { false }
        }

        _ => false,
    }
}


// ---- find_node: fetch node by id (cloned) ----
fn find_node(version: &RuleVersion, node_id: &str) -> Result<Node, StatusCode> {
    version
        .nodes
        .iter()
        .find(|n| n.id == node_id)
        .cloned()
        .ok_or_else(|| {
            error!("Node with id '{}' not found in version '{}'", node_id, version.version);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

// ---- eval_expr: reuse your field-operator-value evaluator on ctx ----
// (Your ctx is a serde_json::Value; expressions like `age >= 18` will look up ctx["age"])
/*fn eval_expr(ctx: &Value, expr: &str) -> bool {
    evaluate_condition(ctx, expr)
}*/
// Evaluate expressions (e.g., `age >= 18`, `country == "MX"`) against flow variables.
fn eval_expr(ctx: &ExecutionContext, expr: &str) -> bool {
    let vars_view = ctx.vars_json();
    evaluate_condition(&vars_view, expr)
}

// ---- ALL: sequential, short-circuit on first fail ----
async fn eval_conditions_all(
    request_id: &str,
    rule_id: &str,
    version: &str,
    conditions: &[RuleCondition],
    ctx: &mut ExecutionContext, // CHANGED: use ExecutionContext
) -> (bool, Vec<ConditionResult>) {
    let client = Client::new();
    let mut results = Vec::with_capacity(conditions.len());

    for cond in conditions {
        let start = Instant::now();

        // interpolate with ctx.vars (supports promoted values like {imsi})
        let url = interpolate(&cond.api, ctx);

        // fetch + parse (null on error)
        let resp_json = match client.get(&url).send().await {
            Ok(resp) => match resp.json::<Value>().await {
                Ok(j) => j,
                Err(_) => Value::Null,
            },
            Err(_) => Value::Null,
        };

        // cache response and promote extracted vars before evaluating
        apply_extracts(ctx, &cond.id, &resp_json, &cond.extract);

        // evaluate condition against the API response JSON
        let ok = evaluate_condition_with_ctx(ctx, &resp_json, &cond.pass_if);

        let total_ms = start.elapsed().as_millis();
        results.push(ConditionResult {
            id: cond.id.clone(),
            passed: ok,
            elapsed_ms: total_ms,
        });

        if !ok {
            return (false, results); // short-circuit on first failure
        }
    }

    (true, results)
}
// ---- ANY: sequential, short-circuit on first success ----
async fn eval_conditions_any(
    request_id: &str,
    rule_id: &str,
    version: &str,
    conditions: &[RuleCondition],
    ctx: &mut ExecutionContext, // CHANGED
) -> (bool, Vec<ConditionResult>) {
    let client = Client::new();
    let mut results = Vec::with_capacity(conditions.len());

    for cond in conditions {
        let start = Instant::now();

        // interpolate with ctx.vars (supports promoted values like {imsi})
        let url = interpolate(&cond.api, ctx);

        // fetch + parse (null on error)
        let resp_json = match client.get(&url).send().await {
            Ok(resp) => match resp.json::<Value>().await {
                Ok(j) => j,
                Err(_) => Value::Null,
            },
            Err(_) => Value::Null,
        };

        // promote extracted vars & cache response before evaluating
        apply_extracts(ctx, &cond.id, &resp_json, &cond.extract);

        // evaluate this condition against the API JSON
        let ok = evaluate_condition_with_ctx(ctx, &resp_json, &cond.pass_if);

        let total_ms = start.elapsed().as_millis();
        results.push(ConditionResult {
            id: cond.id.clone(),
            passed: ok,
            elapsed_ms: total_ms,
        });

        if ok {
            return (true, results); // short-circuit on first success
        }
    }

    (false, results)
}
pub async fn execute_graph(
    request_id: &str,
    rule_id: &str,
    version: &RuleVersion,
    input: &serde_json::Value,
) -> Result<(String, Vec<ConditionResult>), StatusCode> {
    // Use per-request context (vars + cache); seeds vars with input fields
    let mut ctx = ExecutionContext::new(input);

    let mut trace: Vec<ConditionResult> = Vec::new();
    let mut step_guard: usize = 0;
    let max_steps = 200;

    let mut current = find_node(version, "start")?;

    while step_guard < max_steps {
        step_guard += 1;

        // avoid borrowing `current` while we may reassign it
        let kind = current.kind.clone();

        match kind {
            NodeType::Start { next } => {
                current = find_node(version, &next)?;
            }

            NodeType::Case { branches, default } => {
                let mut jumped = false;
                for br in &branches {
                    // evaluate CASE predicates against ctx.vars (includes extracted values)
                    if eval_expr(&ctx, &br.when) {
                        current = find_node(version, &br.next)?;
                        jumped = true;
                        break;
                    }
                }
                if !jumped {
                    current = find_node(version, &default)?;
                }
            }

            NodeType::All { conditions, next_on_pass, next_on_fail } => {
                let (ok, cond_times) =
                    eval_conditions_all(request_id, rule_id, &version.version, &conditions, &mut ctx).await;
                trace.extend(cond_times);

                let next_id: &str = if ok { &next_on_pass } else { &next_on_fail };
                current = find_node(version, next_id)?;
            }

            NodeType::Any { conditions, next_on_pass, next_on_fail } => {
                let (ok, cond_times) =
                    eval_conditions_any(request_id, rule_id, &version.version, &conditions, &mut ctx).await;
                trace.extend(cond_times);

                let next_id: &str = if ok { &next_on_pass } else { &next_on_fail };
                current = find_node(version, next_id)?;
            }

            //TODO: To myself: Implement futures
            NodeType::Action { action, method, url, body, extract, next } => {
                use serde_json::Value;

                let url_i = interpolate(&url, &ctx);
                let body_i = body.as_ref().map(|b| interpolate(b, &ctx));
                let client = reqwest::Client::new();

                let resp_json: Value = match method.as_str() {
                    "POST" => {
                        let send_res = if let Some(b) = &body_i {
                            if let Ok(val) = serde_json::from_str::<Value>(b) {
                                client.post(&url_i).json(&val).send().await
                            } else {
                                client.post(&url_i).body(b.clone()).send().await
                            }
                        } else {
                            client.post(&url_i).send().await
                        };

                        match send_res {
                            Ok(r) => r.json::<Value>().await.unwrap_or(Value::Null),
                            Err(_) => Value::Null,
                        }
                    }

                    "GET" => {
                        match client.get(&url_i).send().await {
                            Ok(r) => r.json::<Value>().await.unwrap_or(Value::Null),
                            Err(_) => Value::Null,
                        }
                    }

                    _ => {
                        // unsupported method in MVP
                        return Err(StatusCode::BAD_REQUEST);
                    }
                };

                // promote extracted fields (e.g., set ctx.vars["imsi"]) and cache under action id
                apply_extracts(&mut ctx, &action, &resp_json, &extract);

                log_file(
                    LOG_PATH,
                    &format!("[{}] action={} {} {} -> vars_now={:?}", request_id, action, method, url_i, ctx.vars),
                );

                current = find_node(version, &next)?;
            }





                /*// Promote extracted fields (e.g., set ctx.vars["imsi"]) and cache under the action name
                apply_extracts(&mut ctx, &action, &resp_json, &extract);

                // (Optional) log
                log_file(LOG_PATH, &format!(
                    "[{}] action={} {} {} -> vars_now={:?}",
                    request_id, action, method, url_i, ctx.vars
                ));

                // Continue
                current = find_node(version, &next)?;
            }*/

            NodeType::Call { target_rule, target_version, bind, result_map, next } => {
                // stub: internal subflow call; for now just log and continue
                log_file(
                    LOG_PATH,
                    &format!(
                        "[{}] call target_rule={} target_version={:?} bind={:?} result_map={:?}",
                        request_id, target_rule, target_version, bind, result_map
                    ),
                );
                // TODO: dispatch to subflow, merge outputs into `ctx`, enforce max_call_depth/timeout
                current = find_node(version, &next)?;
            }

            NodeType::Return { decision } => {
                return Ok((decision, trace));
            }
        }
    }

    // safety fallback (cycle / runaway)
    Ok(("error_max_steps".into(), trace))
}


/*pub async fn execute(rule_id: &str, version: &str) -> String {
    // Placeholder: later this will call actual rule graph or WASM
    format!("Executed {}:{}", rule_id, version)
}*/

pub async fn execute(
    registry: &RuleRegistry,
    rule_id: &str,
    version: &str,
    input: &serde_json::Value,
) -> Result<(String, Vec<ConditionResult>), StatusCode> {
    // Look up the rule
    let rule = registry
        .rules
        .get(rule_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    // Find the version
    let ver = rule
        .versions
        .iter()
        .find(|v| v.version == version)
        .ok_or(StatusCode::NOT_FOUND)?;

    // Execute the rule graph
    execute_graph(
        &uuid::Uuid::new_v4().to_string(),
        rule_id,
        ver,
        input,
    ).await
}

