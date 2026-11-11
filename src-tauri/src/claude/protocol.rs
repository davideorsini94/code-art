//! Parser delle righe stdout di `claude` (stream-json) in eventi tipizzati. Nessun I/O.
//! Formato di riferimento: docs/claude-protocol.md.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AskType {
    Permission,
    Question,
    Plan,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ClaudeEvent {
    Init {
        session_id: String,
        model: String,
        slash_commands: Vec<String>,
        tools: Vec<String>,
        permission_mode: String,
    },
    /// `id` = `<api message id>:<indice blocco>` per i delta in streaming; per il testo di un subagent
    /// (righe `assistant` complete) `id` = id del messaggio.
    TextDelta { id: String, text: String },
    /// Il testo può essere vuoto (la CLI 2.1.319 non lo espone): serve come indicatore "sta pensando".
    Thinking { id: String, text: String },
    ToolStart { id: String, name: String, input: Value, parent_id: Option<String> },
    ToolEnd { id: String, ok: bool, result: String },
    /// Richiesta `can_use_tool`: `tool_name` e `input` servono al motore per rispondere.
    Ask {
        request_id: String,
        ask_type: AskType,
        prompt: String,
        options: Vec<String>,
        tool_name: String,
        input: Value,
    },
    /// La CLI ha annullato una richiesta pendente.
    AskCancelled { request_id: String },
    /// Variazione di un todo ricavata dal risultato di `TaskCreate` / `TaskUpdate` (nessuna riga della CLI
    /// contiene la lista intera: chi consuma la ricostruisce dai delta).
    /// Create: `text` e `status:"pending"`. Update: `status` da `statusChange.to`, `text` assente.
    TodoDelta { id: String, text: Option<String>, status: Option<String> },
    /// `context_used_pct` (0–100) calcolato dall'ultima iterazione API, `None` se la riga non basta.
    TurnEnd { context_used_pct: Option<f32>, is_error: bool },
    /// Risposta della CLI a una `control_request` dell'host.
    ControlResponse { request_id: String, ok: bool, payload: Value },
    /// Qualunque riga non gestita (o JSON non valido). `label` = `type` o `type/subtype`.
    Unknown { label: String, json: String },
}

/// Interpreta una riga stdout. Non fallisce e non va in panico: una riga vuota dà `[]`,
/// una riga non riconosciuta o non valida dà `Unknown`.
pub fn parse_line(line: &str) -> Vec<ClaudeEvent> {
    let line = line.trim();
    if line.is_empty() {
        return vec![];
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![unknown("invalid-json", line)];
    };
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "system" => parse_system(&v, line),
        "stream_event" => parse_stream_event(&v),
        "assistant" => parse_assistant(&v),
        "user" => parse_user(&v, line),
        "result" => vec![parse_result(&v)],
        "control_request" => parse_control_request(&v, line),
        "control_cancel_request" => vec![ClaudeEvent::AskCancelled { request_id: s(&v, "request_id") }],
        "control_response" => vec![parse_control_response(&v)],
        _ => vec![unknown(&label(&v), line)],
    }
}

fn unknown(label: &str, json: &str) -> ClaudeEvent {
    ClaudeEvent::Unknown { label: label.to_string(), json: json.to_string() }
}

fn label(v: &Value) -> String {
    let t = v.get("type").and_then(Value::as_str).unwrap_or("unknown");
    match v.get("subtype").and_then(Value::as_str) {
        Some(sub) => format!("{t}/{sub}"),
        None => t.to_string(),
    }
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn str_list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn parent_id(v: &Value) -> Option<String> {
    v.get("parent_tool_use_id").and_then(Value::as_str).map(String::from)
}

fn parse_system(v: &Value, line: &str) -> Vec<ClaudeEvent> {
    if v.get("subtype").and_then(Value::as_str) != Some("init") {
        return vec![unknown(&label(v), line)];
    }
    vec![ClaudeEvent::Init {
        session_id: s(v, "session_id"),
        model: s(v, "model"),
        slash_commands: str_list(v, "slash_commands"),
        tools: str_list(v, "tools"),
        permission_mode: s(v, "permissionMode"),
    }]
}

/// Solo i delta di testo e thinking; gli altri eventi SSE (start/stop, input parziale degli strumenti,
/// firme) sono rumore già coperto dalle righe `assistant`, quindi non producono nulla.
fn parse_stream_event(v: &Value) -> Vec<ClaudeEvent> {
    let Some(ev) = v.get("event") else { return vec![] };
    if ev.get("type").and_then(Value::as_str) != Some("content_block_delta") {
        return vec![];
    }
    let id = format!("{}:{}", s(v, "api_message_id"), ev.get("index").and_then(Value::as_u64).unwrap_or(0));
    let Some(delta) = ev.get("delta") else { return vec![] };
    match delta.get("type").and_then(Value::as_str) {
        Some("text_delta") => vec![ClaudeEvent::TextDelta { id, text: s(delta, "text") }],
        Some("thinking_delta") => vec![ClaudeEvent::Thinking { id, text: s(delta, "thinking") }],
        _ => vec![],
    }
}

/// Una riga `assistant` porta un solo blocco. I tool_use danno `ToolStart`. Il testo dei blocchi
/// principali è già arrivato dai delta (`--include-partial-messages`) e non si riemette; quello di un
/// subagent no (nessun delta), quindi lo si emette qui.
fn parse_assistant(v: &Value) -> Vec<ClaudeEvent> {
    let Some(msg) = v.get("message") else { return vec![] };
    let parent = parent_id(v);
    let Some(blocks) = msg.get("content").and_then(Value::as_array) else { return vec![] };
    let mut out = vec![];
    for b in blocks {
        match b.get("type").and_then(Value::as_str) {
            Some("tool_use") => out.push(ClaudeEvent::ToolStart {
                id: s(b, "id"),
                name: s(b, "name"),
                input: b.get("input").cloned().unwrap_or(Value::Null),
                parent_id: parent.clone(),
            }),
            Some("text") if parent.is_some() => {
                out.push(ClaudeEvent::TextDelta { id: s(msg, "id"), text: s(b, "text") })
            }
            _ => {}
        }
    }
    out
}

fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn parse_user(v: &Value, line: &str) -> Vec<ClaudeEvent> {
    let blocks = v.pointer("/message/content").and_then(Value::as_array);
    let mut out: Vec<ClaudeEvent> = blocks
        .into_iter()
        .flatten()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
        .map(|b| ClaudeEvent::ToolEnd {
            id: s(b, "tool_use_id"),
            ok: !b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            result: tool_result_text(b.get("content")),
        })
        .collect();
    if out.is_empty() {
        // Replay di messaggi utente, output di comandi locali, marker di interruzione.
        return vec![unknown("user", line)];
    }
    if let Some(d) = todo_delta(v.get("tool_use_result")) {
        out.push(d);
    }
    out
}

/// `TaskCreate` → `task:{id,subject}`; `TaskUpdate` → `taskId` + `statusChange.to`. Un risultato
/// in errore è una stringa e non produce nulla.
fn todo_delta(r: Option<&Value>) -> Option<ClaudeEvent> {
    let r = r?;
    if let Some(task) = r.get("task") {
        let id = task.get("id").and_then(Value::as_str)?;
        let text = task.get("subject").and_then(Value::as_str)?;
        let status = task.get("status").and_then(Value::as_str).unwrap_or("pending");
        return Some(ClaudeEvent::TodoDelta {
            id: id.to_string(),
            text: Some(text.to_string()),
            status: Some(status.to_string()),
        });
    }
    let id = r.get("taskId").and_then(Value::as_str)?;
    let status = r.pointer("/statusChange/to").and_then(Value::as_str)?;
    Some(ClaudeEvent::TodoDelta { id: id.to_string(), text: None, status: Some(status.to_string()) })
}

/// Contesto usato (§9): ultima iterazione = input + cache read + cache creation + output;
/// finestra = `contextWindow` di `modelUsage`, solo se non ambigua (tutti i modelli concordano).
fn context_pct(v: &Value) -> Option<f32> {
    let it = v.pointer("/usage/iterations")?.as_array()?.last()?;
    let n = |k: &str| it.get(k).and_then(Value::as_u64).unwrap_or(0);
    let used = n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens") + n("output_tokens");
    let mut windows = v
        .get("modelUsage")?
        .as_object()?
        .values()
        .filter_map(|m| m.get("contextWindow").and_then(Value::as_u64));
    let w = windows.next()?;
    if w == 0 || windows.any(|x| x != w) {
        return None;
    }
    Some((used as f64 / w as f64 * 100.0) as f32)
}

fn parse_result(v: &Value) -> ClaudeEvent {
    ClaudeEvent::TurnEnd {
        context_used_pct: context_pct(v),
        is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
    }
}

fn parse_control_request(v: &Value, line: &str) -> Vec<ClaudeEvent> {
    let req = v.get("request").unwrap_or(&Value::Null);
    if req.get("subtype").and_then(Value::as_str) != Some("can_use_tool") {
        let sub = req.get("subtype").and_then(Value::as_str).unwrap_or("?");
        return vec![unknown(&format!("control_request/{sub}"), line)];
    }
    let tool_name = s(req, "tool_name");
    let input = req.get("input").cloned().unwrap_or(Value::Null);
    let (ask_type, prompt, options) = match tool_name.as_str() {
        "AskUserQuestion" => {
            let q = input.pointer("/questions/0");
            let prompt = q.map(|q| s(q, "question")).unwrap_or_default();
            let options = q
                .and_then(|q| q.get("options"))
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|o| s(o, "label")).collect())
                .unwrap_or_default();
            (AskType::Question, prompt, options)
        }
        "ExitPlanMode" => (AskType::Plan, s(&input, "plan"), vec![]),
        _ => (AskType::Permission, permission_summary(&tool_name, &input, req), vec![]),
    };
    vec![ClaudeEvent::Ask { request_id: s(v, "request_id"), ask_type, prompt, options, tool_name, input }]
}

fn permission_summary(tool: &str, input: &Value, req: &Value) -> String {
    let arg = ["command", "file_path", "path", "url", "pattern", "query"]
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str))
        .map(String::from)
        .unwrap_or_else(|| s(req, "description"));
    if arg.is_empty() { tool.to_string() } else { format!("{tool}: {arg}") }
}

fn parse_control_response(v: &Value) -> ClaudeEvent {
    let r = v.get("response").unwrap_or(&Value::Null);
    let ok = r.get("subtype").and_then(Value::as_str) == Some("success");
    let payload = if ok {
        r.get("response").cloned().unwrap_or(Value::Null)
    } else {
        json!({ "error": r.get("error").cloned().unwrap_or(Value::Null) })
    };
    ClaudeEvent::ControlResponse { request_id: s(r, "request_id"), ok, payload }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<ClaudeEvent> {
        let path = format!("{}/tests/fixtures/stream/{name}.jsonl", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(path).unwrap().lines().flat_map(parse_line).collect()
    }

    fn kinds(evs: &[ClaudeEvent]) -> Vec<&'static str> {
        evs.iter().map(kind_of).collect()
    }

    fn kind_of(e: &ClaudeEvent) -> &'static str {
        match e {
            ClaudeEvent::Init { .. } => "init",
            ClaudeEvent::TextDelta { .. } => "textDelta",
            ClaudeEvent::Thinking { .. } => "thinking",
            ClaudeEvent::ToolStart { .. } => "toolStart",
            ClaudeEvent::ToolEnd { .. } => "toolEnd",
            ClaudeEvent::Ask { .. } => "ask",
            ClaudeEvent::AskCancelled { .. } => "askCancelled",
            ClaudeEvent::TodoDelta { .. } => "todoDelta",
            ClaudeEvent::TurnEnd { .. } => "turnEnd",
            ClaudeEvent::ControlResponse { .. } => "controlResponse",
            ClaudeEvent::Unknown { .. } => "unknown",
        }
    }

    fn asks(evs: &[ClaudeEvent]) -> Vec<&ClaudeEvent> {
        evs.iter().filter(|e| matches!(e, ClaudeEvent::Ask { .. })).collect()
    }

    #[test]
    fn simple_turn_is_init_deltas_turnend() {
        let evs = fixture("simple-turn");
        let k: Vec<_> = kinds(&evs).into_iter().filter(|k| *k != "unknown").collect();
        assert_eq!(k, ["init", "textDelta", "turnEnd"]);
        let ClaudeEvent::Init { session_id, model, slash_commands, tools, permission_mode } =
            evs.iter().find(|e| matches!(e, ClaudeEvent::Init { .. })).expect("init")
        else {
            unreachable!()
        };
        assert!(!session_id.is_empty() && model.starts_with("claude-"));
        assert!(!slash_commands.is_empty() && !tools.is_empty());
        assert_eq!(permission_mode, "default");
        let text: String = evs
            .iter()
            .filter_map(|e| if let ClaudeEvent::TextDelta { text, .. } = e { Some(text.as_str()) } else { None })
            .collect();
        assert!(!text.is_empty());
        let ClaudeEvent::TurnEnd { context_used_pct, is_error } = evs.last().unwrap() else { panic!() };
        assert!(!is_error);
        let pct = context_used_pct.expect("simple-turn has iterations and a single model");
        assert!((pct - 3.29).abs() < 0.05, "pct = {pct}"); // (2+27860+5067+4) / 1_000_000
    }

    #[test]
    fn permission_request_fixture_has_permission_ask() {
        let evs = fixture("permission-request");
        let a = asks(&evs);
        assert_eq!(a.len(), 3);
        let ClaudeEvent::Ask { ask_type, prompt, options, tool_name, input, request_id } = a[0] else { panic!() };
        assert_eq!(*ask_type, AskType::Permission);
        assert_eq!(tool_name, "Write");
        assert!(prompt.starts_with("Write: ") && prompt.ends_with("note.txt"), "{prompt}");
        assert!(options.is_empty() && !request_id.is_empty());
        assert!(input.get("file_path").is_some());
        let ClaudeEvent::Ask { prompt, .. } = a[1] else { panic!() };
        assert!(prompt.starts_with("Bash: touch"), "{prompt}");
        // il risultato negato è un tool_result in errore
        assert!(evs.iter().any(|e| matches!(e, ClaudeEvent::ToolEnd { ok: false, .. })));
        assert_eq!(kinds(&evs).iter().filter(|k| **k == "turnEnd").count(), 3);
    }

    #[test]
    fn question_fixture_has_options_from_first_question() {
        let evs = fixture("ask-user-question");
        let a = asks(&evs);
        assert_eq!(a.len(), 1);
        let ClaudeEvent::Ask { ask_type, prompt, options, tool_name, .. } = a[0] else { panic!() };
        assert_eq!(*ask_type, AskType::Question);
        assert_eq!(tool_name, "AskUserQuestion");
        assert_eq!(prompt, "Which color do you prefer?");
        assert_eq!(options, &["Red", "Blue"]);
    }

    #[test]
    fn plan_fixture_has_plan_ask_with_plan_text() {
        let evs = fixture("plan-mode");
        let plan = evs
            .iter()
            .find_map(|e| match e {
                ClaudeEvent::Ask { ask_type: AskType::Plan, prompt, tool_name, .. } => Some((prompt, tool_name)),
                _ => None,
            })
            .expect("plan ask");
        assert_eq!(plan.1, "ExitPlanMode");
        assert!(plan.0.starts_with("# Plan"), "{}", plan.0);
        assert!(evs.iter().any(|e| matches!(e, ClaudeEvent::Thinking { .. })));
    }

    #[test]
    fn subagent_tools_carry_parent_id() {
        let evs = fixture("subagent");
        let starts: Vec<_> = evs
            .iter()
            .filter_map(|e| match e {
                ClaudeEvent::ToolStart { name, parent_id, id, .. } => Some((name.as_str(), parent_id.clone(), id.clone())),
                _ => None,
            })
            .collect();
        let agent = starts.iter().find(|s| s.0 == "Agent").expect("Agent tool");
        assert_eq!(agent.1, None);
        let child = starts.iter().find(|s| s.0 == "Read").expect("child tool");
        assert_eq!(child.1.as_deref(), Some(agent.2.as_str()));
        // il testo del subagent arriva dalla riga completa (nessun delta)
        assert!(evs.iter().any(|e| matches!(e, ClaudeEvent::TextDelta { text, .. } if text == "color = blue")));
        // due result: il secondo non è stato richiesto dall'host
        assert_eq!(kinds(&evs).iter().filter(|k| **k == "turnEnd").count(), 2);
    }

    #[test]
    fn todo_fixture_yields_deltas() {
        let evs = fixture("todo-update");
        let d: Vec<_> = evs
            .iter()
            .filter_map(|e| match e {
                ClaudeEvent::TodoDelta { id, text, status } => Some((id.as_str(), text.as_deref(), status.as_deref())),
                _ => None,
            })
            .collect();
        assert_eq!(
            d,
            [
                ("1", Some("Write intro"), Some("pending")),
                ("2", Some("Write outro"), Some("pending")),
                ("1", None, Some("in_progress")),
                ("1", None, Some("completed")),
            ]
        );
    }

    #[test]
    fn tool_use_edit_fixture_pairs_start_and_end() {
        let evs = fixture("tool-use-edit");
        let names: Vec<_> = evs
            .iter()
            .filter_map(|e| if let ClaudeEvent::ToolStart { name, .. } = e { Some(name.as_str()) } else { None })
            .collect();
        assert_eq!(names, ["Read", "Edit"]);
        let ends = evs.iter().filter(|e| matches!(e, ClaudeEvent::ToolEnd { ok: true, .. })).count();
        assert_eq!(ends, 2);
    }

    #[test]
    fn interrupt_fixture_ends_with_error_turn() {
        let evs = fixture("interrupt");
        assert!(evs.iter().any(|e| matches!(e, ClaudeEvent::TurnEnd { is_error: true, .. })));
        assert!(evs.iter().any(|e| matches!(e, ClaudeEvent::ControlResponse { ok: true, .. })));
    }

    #[test]
    fn control_fixture_has_cancel_and_error_response() {
        let evs = fixture("control-misc");
        let cancel = evs.iter().find_map(|e| match e {
            ClaudeEvent::AskCancelled { request_id } => Some(request_id.as_str()),
            _ => None,
        });
        assert_eq!(cancel, Some("8cb99aec-2f6c-4455-8e23-32e3cdc26490"));
        let err = evs
            .iter()
            .find_map(|e| match e {
                ClaudeEvent::ControlResponse { request_id, ok: false, payload } => Some((request_id, payload)),
                _ => None,
            })
            .expect("error response");
        assert_eq!(err.0, "host_req_3");
        assert!(err.1["error"].as_str().unwrap().contains("no_such_thing"));
    }

    #[test]
    fn slash_command_fixture_has_no_ambiguous_context() {
        // /compact produce un result con num_turns 0 e modelUsage vuoto: nessuna percentuale.
        let evs = fixture("slash-command");
        assert!(evs.iter().any(|e| matches!(e, ClaudeEvent::TurnEnd { context_used_pct: None, .. })));
        assert!(evs.iter().any(|e| matches!(e, ClaudeEvent::Unknown { label, .. } if label == "system/compact_boundary")));
    }

    #[test]
    fn model_switch_fixture_parses_responses_and_two_inits() {
        let evs = fixture("model-switch");
        let models: Vec<_> = evs
            .iter()
            .filter_map(|e| if let ClaudeEvent::Init { model, permission_mode, .. } = e { Some((model.as_str(), permission_mode.as_str())) } else { None })
            .collect();
        assert_eq!(models.len(), 2);
        assert_ne!(models[0].0, models[1].0);
        assert_eq!(models[1].1, "acceptEdits");
        assert_eq!(evs.iter().filter(|e| matches!(e, ClaudeEvent::ControlResponse { .. })).count(), 2);
    }

    #[test]
    fn image_input_fixture_parses() {
        let evs = fixture("image-input");
        assert_eq!(kinds(&evs).iter().filter(|k| **k == "turnEnd").count(), 1);
    }

    #[test]
    fn unknown_fixture_keeps_future_line_as_unknown() {
        let evs = fixture("unknown");
        assert!(evs.iter().any(|e| matches!(e, ClaudeEvent::Unknown { label, .. } if label == "future-thing")));
        assert!(evs.iter().any(|e| matches!(e, ClaudeEvent::TurnEnd { .. })));
    }

    #[test]
    fn garbage_line_is_unknown_not_panic() {
        let evs = parse_line("{not json");
        assert_eq!(evs, [ClaudeEvent::Unknown { label: "invalid-json".into(), json: "{not json".into() }]);
        let evs = parse_line(r#"{"type":"future-thing"}"#);
        assert_eq!(evs, [ClaudeEvent::Unknown { label: "future-thing".into(), json: r#"{"type":"future-thing"}"#.into() }]);
        // JSON valido ma di forma inattesa
        for l in ["42", "null", "[]", r#"{"type":"assistant"}"#, r#"{"type":"user","message":5}"#, r#"{"type":"control_request"}"#, r#"{"type":"result"}"#] {
            let _ = parse_line(l);
        }
        assert!(parse_line("").is_empty() && parse_line("  \n").is_empty());
    }

    #[test]
    fn unhandled_lines_keep_type_and_subtype_label() {
        let evs = parse_line(r#"{"type":"system","subtype":"status","status":"requesting"}"#);
        assert!(matches!(&evs[0], ClaudeEvent::Unknown { label, .. } if label == "system/status"));
        let evs = parse_line(r#"{"type":"user","message":{"role":"user","content":"hi"},"isReplay":true}"#);
        assert!(matches!(&evs[0], ClaudeEvent::Unknown { label, .. } if label == "user"));
    }

    #[test]
    fn tool_result_blocks_array_are_joined_and_errors_flagged() {
        let l = r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"a"},{"type":"image"},{"type":"text","text":"b"}]},
            {"type":"tool_result","tool_use_id":"t2","is_error":true,"content":"boom"}]},"tool_use_result":"Error: boom"}"#
            .replace('\n', "");
        let evs = parse_line(&l);
        assert_eq!(
            evs,
            [
                ClaudeEvent::ToolEnd { id: "t1".into(), ok: true, result: "a\nb".into() },
                ClaudeEvent::ToolEnd { id: "t2".into(), ok: false, result: "boom".into() },
            ]
        );
    }

    #[test]
    fn one_assistant_line_can_yield_several_events() {
        let l = r#"{"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"a","name":"Read","input":{}},{"type":"tool_use","id":"b","name":"Bash","input":{"command":"ls"}}]},"parent_tool_use_id":null}"#;
        assert_eq!(kinds(&parse_line(l)), ["toolStart", "toolStart"]);
    }

    #[test]
    fn context_pct_is_none_when_models_disagree_on_window() {
        let l = r#"{"type":"result","is_error":false,"usage":{"iterations":[{"input_tokens":100,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}]},"modelUsage":{"a":{"contextWindow":1000},"b":{"contextWindow":2000}}}"#;
        assert_eq!(parse_line(l), [ClaudeEvent::TurnEnd { context_used_pct: None, is_error: false }]);
        let l = l.replace("2000", "1000");
        assert_eq!(parse_line(&l), [ClaudeEvent::TurnEnd { context_used_pct: Some(10.0), is_error: false }]);
    }

    #[test]
    fn serialized_kind_strings_match_the_typescript_mirror() {
        let all = vec![
            ClaudeEvent::Init { session_id: "s".into(), model: "m".into(), slash_commands: vec![], tools: vec![], permission_mode: "default".into() },
            ClaudeEvent::TextDelta { id: "i".into(), text: "t".into() },
            ClaudeEvent::Thinking { id: "i".into(), text: "t".into() },
            ClaudeEvent::ToolStart { id: "i".into(), name: "n".into(), input: json!({}), parent_id: None },
            ClaudeEvent::ToolEnd { id: "i".into(), ok: true, result: "r".into() },
            ClaudeEvent::Ask { request_id: "r".into(), ask_type: AskType::Plan, prompt: "p".into(), options: vec![], tool_name: "t".into(), input: json!({}) },
            ClaudeEvent::AskCancelled { request_id: "r".into() },
            ClaudeEvent::TodoDelta { id: "1".into(), text: None, status: Some("pending".into()) },
            ClaudeEvent::TurnEnd { context_used_pct: Some(1.0), is_error: false },
            ClaudeEvent::ControlResponse { request_id: "r".into(), ok: true, payload: json!(null) },
            ClaudeEvent::Unknown { label: "l".into(), json: "{}".into() },
        ];
        let got: Vec<String> = all.iter().map(|e| serde_json::to_value(e).unwrap()["kind"].as_str().unwrap().to_string()).collect();
        assert_eq!(
            got,
            ["init", "textDelta", "thinking", "toolStart", "toolEnd", "ask", "askCancelled", "todoDelta", "turnEnd", "controlResponse", "unknown"]
        );
        // campi in camelCase
        let v = serde_json::to_value(&all[0]).unwrap();
        assert!(v.get("sessionId").is_some() && v.get("permissionMode").is_some() && v.get("slashCommands").is_some());
        let v = serde_json::to_value(&all[5]).unwrap();
        assert_eq!(v["askType"], "plan");
        assert!(v.get("requestId").is_some() && v.get("toolName").is_some());
        assert!(serde_json::to_value(&all[3]).unwrap().get("parentId").is_some());
        assert!(serde_json::to_value(&all[8]).unwrap().get("contextUsedPct").is_some());
    }
}
