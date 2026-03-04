use crate::models::{ChatRequest, ChatResponse};
use crate::state::{AppState, ControlRequest};
use axum::{
    Json,
    extract::State,
    response::sse::{Event, Sse},
};
use futures_util::StreamExt;
use mad_core::ControlCommand;
use mad_core::{AccessibilityScanner, ScreenCapture, StreamChunk};
use mad_skills::DesktopAction;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::time::{Duration, timeout};
use tracing::{error, info};

const COMMAND_SEND_TIMEOUT_MS: u64 = 1200;
const UI_TREE_TIMEOUT_MS: u64 = 2200;
const SCREENSHOT_TIMEOUT_MS: u64 = 2200;
const COMPONENT_LOOKUP_TIMEOUT_MS: u64 = 1600;

async fn send_control_command(state: &AppState, cmd: ControlCommand) -> Result<(), String> {
    let cmd_debug = format!("{:?}", cmd);
    let completion_timeout = match &cmd {
        ControlCommand::Wait(seconds) => Duration::from_secs(seconds.saturating_add(3)),
        _ => {
            let ms = std::env::var("MAD_COMMAND_EXEC_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(8000);
            Duration::from_millis(ms)
        }
    };

    let (done_tx, done_rx) = oneshot::channel();
    let request = ControlRequest { cmd, done_tx };

    match timeout(
        Duration::from_millis(COMMAND_SEND_TIMEOUT_MS),
        state.command_tx.send(request),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(format!("Failed to send command {}: {}", cmd_debug, e)),
        Err(_) => return Err(format!("Command queue timeout for {}", cmd_debug)),
    }

    match timeout(completion_timeout, done_rx).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(e))) => Err(format!("Command {} execution failed: {}", cmd_debug, e)),
        Ok(Err(_)) => Err(format!("Command {} completion channel dropped", cmd_debug)),
        Err(_) => Err(format!(
            "Command {} execution timeout after {:?}",
            cmd_debug, completion_timeout
        )),
    }
}

async fn capture_ui_tree_with_timeout(detailed: bool) -> Result<String, String> {
    let allow_detailed_inspect = std::env::var("MAD_ENABLE_DETAILED_INSPECT")
        .ok()
        .map(|v| {
            let lower = v.to_ascii_lowercase();
            lower == "1" || lower == "true" || lower == "yes" || lower == "on"
        })
        .unwrap_or(false);
    let capture_detailed = detailed && allow_detailed_inspect;
    let timeout_ms = std::env::var("MAD_UI_TREE_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(UI_TREE_TIMEOUT_MS);
    let task = tokio::task::spawn_blocking(move || {
        AccessibilityScanner::capture_active_window_tree(capture_detailed)
    });
    match timeout(Duration::from_millis(timeout_ms), task).await {
        Ok(Ok(Ok(tree))) => Ok(tree),
        Ok(Ok(Err(e))) => Err(format!("UI tree error: {}", e)),
        Ok(Err(e)) => Err(format!("UI tree task join error: {}", e)),
        Err(_) => Err(format!("UI tree timeout after {}ms", timeout_ms)),
    }
}

async fn capture_screenshot_with_timeout() -> Result<String, String> {
    let timeout_ms = std::env::var("MAD_SCREENSHOT_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(SCREENSHOT_TIMEOUT_MS);
    let task = tokio::task::spawn_blocking(ScreenCapture::capture_main_base64);
    match timeout(Duration::from_millis(timeout_ms), task).await {
        Ok(Ok(Ok(image))) => Ok(image),
        Ok(Ok(Err(e))) => Err(format!("Screenshot error: {}", e)),
        Ok(Err(e)) => Err(format!("Screenshot task join error: {}", e)),
        Err(_) => Err(format!("Screenshot timeout after {}ms", timeout_ms)),
    }
}

async fn find_component_center_with_timeout(text: &str) -> Result<Option<(i32, i32)>, String> {
    let timeout_ms = std::env::var("MAD_COMPONENT_LOOKUP_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(COMPONENT_LOOKUP_TIMEOUT_MS);
    let query = text.to_string();
    let task =
        tokio::task::spawn_blocking(move || AccessibilityScanner::find_element_center(&query));
    match timeout(Duration::from_millis(timeout_ms), task).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(e)) => Err(format!("Component lookup task join error: {}", e)),
        Err(_) => Err(format!("Component lookup timeout after {}ms", timeout_ms)),
    }
}

fn extract_text_payload(action: &Value) -> Option<String> {
    for key in ["text", "content", "value", "input", "message", "prompt"] {
        if let Some(v) = action.get(key).and_then(|v| v.as_str()) {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn is_generic_text_target(text: &str) -> bool {
    matches!(
        text.to_ascii_lowercase().as_str(),
        "textarea" | "text area" | "text_field" | "textfield" | "input" | "editor"
    )
}

async fn execute_action_value(state: &AppState, action: &Value) -> Result<String, String> {
    // 1. Try strict parsing
    if let Ok(desktop_action) = serde_json::from_value::<DesktopAction>(action.clone()) {
        if let DesktopAction::Inspect(_) = &desktop_action {
            return Ok("Inspect".to_string());
        }

        if let DesktopAction::ClickComponent(args) = &desktop_action {
            if let Some((x, y)) = find_component_center_with_timeout(&args.text).await? {
                send_control_command(state, ControlCommand::MoveMouse(x, y)).await?;
                send_control_command(state, ControlCommand::Click).await?;
                return Ok(format!("ClickComponent '{}' at ({}, {})", args.text, x, y));
            }
            if is_generic_text_target(&args.text) {
                return Ok(format!(
                    "ClickComponent '{}' not found, skip and continue typing",
                    args.text
                ));
            }
            return Err(format!("Component '{}' not found", args.text));
        }

        let commands = desktop_action.to_commands();
        let desc = format!("Queued {:?}", commands);
        for cmd in commands {
            send_control_command(state, cmd).await?;
        }
        return Ok(desc);
    }

    // 2. Fallback: Manual Loose Parsing (Migration path)
    let act_type = action["action"]
        .as_str()
        .or_else(|| action["type"].as_str());

    if let Some(act_type) = act_type {
        match act_type {
            "click" | "mouse_move" | "move_mouse" => {
                let x = action["x"]
                    .as_i64()
                    .or_else(|| action["x"].as_f64().map(|v| v as i64));
                let y = action["y"]
                    .as_i64()
                    .or_else(|| action["y"].as_f64().map(|v| v as i64));
                // Handle nested "pos"
                let (x, y) = if x.is_none() || y.is_none() {
                    if let Some(pos) = action.get("pos") {
                        let px = pos["x"]
                            .as_i64()
                            .or_else(|| pos["x"].as_f64().map(|v| v as i64));
                        let py = pos["y"]
                            .as_i64()
                            .or_else(|| pos["y"].as_f64().map(|v| v as i64));
                        (px, py)
                    } else {
                        (None, None)
                    }
                } else {
                    (x, y)
                };

                if let (Some(x), Some(y)) = (x, y) {
                    send_control_command(state, ControlCommand::MoveMouse(x as i32, y as i32))
                        .await?;
                    if act_type == "click" {
                        send_control_command(state, ControlCommand::Click).await?;
                        return Ok(format!("Click {}, {}", x, y));
                    }
                    return Ok(format!("Move {}, {}", x, y));
                }
            }
            "type" | "text" | "input" | "write" => {
                if let Some(text) = extract_text_payload(action) {
                    if text.len() > 50 || text.starts_with("http") {
                        send_control_command(state, ControlCommand::Paste(text.clone())).await?;
                    } else {
                        send_control_command(state, ControlCommand::Type(text.clone())).await?;
                    }
                    return Ok(format!("Type '{}'", text));
                }
            }
            "key" | "hotkey" | "press" | "shortcut" => {
                let keys = extract_keys(action);
                if !keys.is_empty() {
                    send_control_command(state, ControlCommand::KeySequence(keys.clone())).await?;
                    return Ok(format!("Key {:?}", keys));
                }
            }
            "select_all" => {
                let keys = vec!["Command".to_string(), "A".to_string()];
                send_control_command(state, ControlCommand::KeySequence(keys.clone())).await?;
                return Ok(format!("Key {:?}", keys));
            }
            "delete" | "clear" => {
                let keys = vec!["Backspace".to_string()];
                send_control_command(state, ControlCommand::KeySequence(keys.clone())).await?;
                return Ok(format!("Key {:?}", keys));
            }
            "wait" | "delay" => {
                let seconds = action["seconds"]
                    .as_u64()
                    .or_else(|| action["duration"].as_u64());
                if let Some(seconds) = seconds {
                    send_control_command(state, ControlCommand::Wait(seconds)).await?;
                    return Ok(format!("Wait {}s", seconds));
                }
            }
            "minimize" => {
                send_control_command(state, ControlCommand::Minimize).await?;
                return Ok("Minimize".to_string());
            }
            "screenshot" => {
                send_control_command(state, ControlCommand::Screenshot).await?;
                return Ok("Screenshot".to_string());
            }
            "inspect" => {
                return Ok("Inspect".to_string());
            }
            "click_component" => {
                if let Some(text) = action
                    .get("text")
                    .or_else(|| action.get("target"))
                    .or_else(|| action.get("name"))
                    .and_then(|v| v.as_str())
                {
                    if let Some((x, y)) = find_component_center_with_timeout(text).await? {
                        send_control_command(state, ControlCommand::MoveMouse(x, y)).await?;
                        send_control_command(state, ControlCommand::Click).await?;
                        return Ok(format!("ClickComponent '{}' at ({}, {})", text, x, y));
                    }
                    if is_generic_text_target(text) {
                        return Ok(format!(
                            "ClickComponent '{}' not found, skip and continue typing",
                            text
                        ));
                    }
                    return Err(format!("Component '{}' not found", text));
                }
            }
            _ => {}
        }
    }

    Ok("Skip unsupported action".to_string())
}

fn extract_keys(action: &serde_json::Value) -> Vec<String> {
    // Try "keys" array
    if let Some(arr) = action.get("keys").and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }

    // Try "keys" string (e.g. "command+space")
    if let Some(s) = action.get("keys").and_then(|v| v.as_str()) {
        return s.split('+').map(|k| k.trim().to_string()).collect();
    }

    // Try "key" string or array
    if let Some(val) = action.get("key") {
        if let Some(arr) = val.as_array() {
            return arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(s) = val.as_str() {
            return s.split('+').map(|k| k.trim().to_string()).collect();
        }
    }

    Vec::new()
}

pub async fn chat_handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Json<ChatResponse> {
    let response = process_chat(state, req.instruction).await;
    Json(response)
}

fn format_memories(memories: &[(crate::memory::MemoryItem, f32)]) -> String {
    if memories.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "\n\nRELEVANT PAST EXPERIENCES (Use these to avoid mistakes and plan better):\n",
    );
    for (mem, score) in memories {
        if *score > 0.5 {
            s.push_str(&format!("- Past Instruction: \"{}\"\n", mem.content));
            if let Some(thoughts) = mem.metadata.get("thoughts").and_then(|v| v.as_str()) {
                s.push_str(&format!("  My Thoughts: {}\n", thoughts));
            }
            if let Some(actions) = mem.metadata.get("actions") {
                s.push_str(&format!("  Actions Taken: {}\n", actions));
            }
            s.push('\n');
        }
    }
    s
}

fn parse_actions_from_value(value: Value) -> Option<Vec<Value>> {
    match value {
        Value::Object(mut map) => {
            if let Some(actions) = map.remove("actions").and_then(|v| v.as_array().cloned()) {
                return Some(actions);
            }
            if map.contains_key("action") || map.contains_key("type") {
                return Some(vec![Value::Object(map)]);
            }
            None
        }
        Value::Array(actions) => Some(actions),
        _ => None,
    }
}

fn parse_response_json(response: &str) -> Option<Value> {
    let trimmed = response.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }

    if let Some(fence_start) = trimmed.find("```") {
        let fenced = &trimmed[fence_start + 3..];
        if let Some(fence_end) = fenced.find("```") {
            let candidate = fenced[..fence_end].trim_start_matches("json").trim();
            if let Ok(value) = serde_json::from_str::<Value>(candidate) {
                return Some(value);
            }
        }
    }

    let mut candidates = collect_balanced_json_segments(trimmed, '{', '}');
    candidates.extend(collect_balanced_json_segments(trimmed, '[', ']'));
    candidates.sort_by_key(|s| std::cmp::Reverse(s.len()));

    for candidate in candidates {
        if let Ok(value) = serde_json::from_str::<Value>(&candidate) {
            return Some(value);
        }
    }

    None
}

fn parse_completion_meta(response: &str) -> (Option<bool>, Option<String>, Option<String>) {
    let Some(value) = parse_response_json(response) else {
        return (None, None, None);
    };
    let Value::Object(map) = value else {
        return (None, None, None);
    };

    let completed = map
        .get("completed")
        .or_else(|| map.get("done"))
        .or_else(|| map.get("success"))
        .and_then(|v| v.as_bool());
    let done_reason = map
        .get("done_reason")
        .or_else(|| map.get("result"))
        .or_else(|| map.get("outcome"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let success_check = map
        .get("success_criteria_check")
        .or_else(|| map.get("verification"))
        .or_else(|| map.get("status"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    (completed, done_reason, success_check)
}

fn has_high_risk_approval(instruction: &str) -> bool {
    let lower = instruction.to_ascii_lowercase();
    lower.contains("#confirm-risk")
        || instruction.contains("确认高风险操作")
        || instruction.contains("允许高风险操作")
}

fn is_high_risk_action(action: &Value) -> bool {
    let act_type = action["action"]
        .as_str()
        .or_else(|| action["type"].as_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if matches!(
        act_type.as_str(),
        "delete" | "clear" | "minimize" | "key" | "hotkey" | "press" | "shortcut"
    ) {
        return true;
    }

    let text = extract_text_payload(action)
        .unwrap_or_default()
        .to_ascii_lowercase();
    !text.is_empty()
        && ["password", "token", "api_key", "secret", "密钥", "密码"]
            .iter()
            .any(|k| text.contains(k))
}

fn build_high_risk_block_message(action: &Value) -> String {
    let act_type = action["action"]
        .as_str()
        .or_else(|| action["type"].as_str())
        .unwrap_or("unknown");
    format!(
        "Blocked high-risk action '{}'. Add '#confirm-risk' or '确认高风险操作' in instruction to continue.",
        act_type
    )
}

fn collect_balanced_json_segments(input: &str, open: char, close: char) -> Vec<String> {
    let mut segments = Vec::new();
    let mut depth = 0usize;
    let mut start_idx: Option<usize> = None;
    let mut in_string = false;
    let mut escaped = false;

    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            continue;
        }

        if ch == open {
            if depth == 0 {
                start_idx = Some(idx);
            }
            depth += 1;
            continue;
        }

        if ch == close && depth > 0 {
            depth -= 1;
            if depth == 0 {
                if let Some(start) = start_idx {
                    segments.push(input[start..=idx].to_string());
                }
                start_idx = None;
            }
        }
    }

    segments
}

fn build_inference_messages(messages: &[Value], max_non_system_messages: usize) -> Vec<Value> {
    if messages.len() <= max_non_system_messages + 1 {
        return messages.to_vec();
    }

    let mut trimmed = Vec::with_capacity(max_non_system_messages + 1);
    trimmed.push(messages[0].clone());
    let start = messages.len().saturating_sub(max_non_system_messages);
    trimmed.extend(messages[start..].iter().cloned());
    trimmed
}

pub async fn process_chat(state: AppState, instruction: String) -> ChatResponse {
    info!("Received instruction: {}", instruction);
    let instruction_for_memory = instruction.clone();
    let vision_model =
        std::env::var("MAD_VISION_MODEL").unwrap_or_else(|_| "deepseek-vl".to_string());

    // 0. Retrieve Memories
    let memory_context = {
        let mut context = String::new();
        if let Some(mem_store) = &state.memory {
            let mut mem_guard = mem_store.lock().await;
            if let Ok(memories) = mem_guard.search(&instruction, 3) {
                context = format_memories(&memories);
            }
        }
        context
    };

    // 1. Determine if we need a screenshot
    let is_text_only_model = state.model == "deepseek-chat" || state.model == "deepseek-reasoner";

    // 2. Construct Message for Deepseek
    let messages = if is_text_only_model {
        let ui_tree = capture_ui_tree_with_timeout(true)
            .await
            .unwrap_or_else(|e| format!("Error capturing UI: {}", e));

        let os = std::env::consts::OS;
        let system_content = format!(
            "You are an expert computer control agent running on {}. You receive user instructions and the current UI structure.\n\n\
            GOAL: Complete the user's task by navigating the UI.\n\n\
            RESPONSE FORMAT:\n\
            {{\n\
              \"thoughts\": \"reasoning...\",\n\
              \"completed\": false,\n\
              \"done_reason\": \"why still running or why done\",\n\
              \"success_criteria_check\": \"what has been verified\",\n\
              \"actions\": [\n\
                {{ \"action\": \"key\", \"keys\": [\"Command\", \"Space\"] }},\n\
                {{ \"action\": \"type\", \"text\": \"safari\" }}\n\
              ]\n\
            }}\n\n\
            {}\n\n\
            {}\n\n\
            RULES:\n\
            1. If you need to interact with other apps, ALWAYS start with {{ \"action\": \"minimize\" }}.\n\
            2. Prefer KEYBOARD SHORTCUTS over mouse clicks.\n\
            3. UI Tree Format: - [Role] \"Title\" (Description) val:Value @ [x, y, width, height].\n\
            4. If task is done, set \"completed\": true and return empty \"actions\".\n\
            5. Output valid JSON only.",
            os,
            DesktopAction::description(),
            memory_context
        );

        vec![
            json!({
                "role": "system",
                "content": system_content
            }),
            json!({
                "role": "user",
                "content": format!("Instruction: {}\n\nCurrent UI State:\n{}", instruction, ui_tree)
            }),
        ]
    } else {
        let b64_image = match capture_screenshot_with_timeout().await {
            Ok(img) => img,
            Err(e) => {
                return ChatResponse {
                    message: format!("Failed to capture screen: {}", e),
                    actions_performed: vec![],
                };
            }
        };

        let system_content = format!(
            "You are an expert computer control agent. You receive a screenshot and a user instruction.\n\n\
            GOAL: Complete the user's task by outputting a JSON object with 'thoughts' and 'actions'.\n\n\
            RESPONSE FORMAT: {{\"thoughts\": \"reasoning...\", \"completed\": false, \"done_reason\": \"...\", \"success_criteria_check\": \"...\", \"actions\": [{{...}}]}}\n\n\
            {}\n\n\
            {}\n\n\
            RULES:\n\
            1. If you need to click, use coordinates from the screenshot (resized to logical points).\n\
            2. If task is done, set \"completed\": true and return empty \"actions\".\n\
            3. Output valid JSON only.",
            DesktopAction::description(),
            memory_context
        );

        vec![
            json!({
                "role": "system",
                "content": system_content
            }),
            json!({
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": instruction
                    },
                    {
                        "type": "image_url",
                        "image_url": {
                            "url": format!("data:image/png;base64,{}", b64_image)
                        }
                    }
                ]
            }),
        ]
    };

    // 3. Call Deepseek
    let ai_response = if is_text_only_model {
        state.client.chat(messages).await
    } else {
        state
            .client
            .chat_with_model(messages, Some(vision_model.as_str()))
            .await
    };
    let ai_response = match ai_response {
        Ok(res) => res,
        Err(e) => {
            return ChatResponse {
                message: format!("AI Error: {}", e),
                actions_performed: vec![],
            };
        }
    };

    info!("AI Response: {}", ai_response);

    // Save Memory
    let state_clone = state.clone();
    let instruction_clone = instruction_for_memory;
    let response_clone = ai_response.clone();

    tokio::spawn(async move {
        // Parse metadata
        let metadata = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response_clone)
        {
            json!({
                "thoughts": json.get("thoughts"),
                "actions": json.get("actions")
            })
        } else {
            json!({ "raw_response": response_clone })
        };

        // Add to memory
        if let Some(mem_store) = &state_clone.memory {
            let mut mem = mem_store.lock().await;
            if let Err(e) = mem.add_memory(&instruction_clone, metadata) {
                error!("Failed to save memory: {}", e);
            }
        }
    });

    // 4. Parse Actions and Execute
    let mut actions_log = Vec::new();
    let actions = parse_actions(&ai_response);
    let high_risk_approved = has_high_risk_approval(&instruction);

    for action in actions {
        if is_high_risk_action(&action) && !high_risk_approved {
            let blocked = build_high_risk_block_message(&action);
            actions_log.push(format!("BLOCKED: {}", blocked));
            continue;
        }
        match execute_action_value(&state, &action).await {
            Ok(desc) => actions_log.push(desc),
            Err(e) => error!("Action execution failed: {}", e),
        }
    }

    ChatResponse {
        message: ai_response,
        actions_performed: actions_log,
    }
}

pub async fn chat_stream_handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, axum::Error>>> {
    info!("Received streaming instruction: {}", req.instruction);

    let stream = async_stream::stream! {
        let is_text_only_model = state.model == "deepseek-chat" || state.model == "deepseek-reasoner";
        let vision_model = std::env::var("MAD_VISION_MODEL").unwrap_or_else(|_| "deepseek-vl".to_string());
        let max_steps = std::env::var("MAD_MAX_STEPS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(10);
        let mut messages = Vec::new();
        let mut detailed_mode = false;
        let mut capture_screenshot = false;

        // 1. System Prompt
        let system_content = if is_text_only_model {
            format!(
                "You are an expert computer control agent. You receive user instructions and the current UI structure (Accessibility Tree) of the active window.\n\n\
                GOAL: Complete the user's task by navigating the UI.\n\n\
                RESPONSE FORMAT:\n\
                Output a JSON object with a 'thoughts' field and an 'actions' array.\n\
                {{\n\
                  \"thoughts\": \"Detailed reasoning about what you see and what you will do next...\",\n\
                  \"completed\": false,\n\
                  \"done_reason\": \"why still running or why done\",\n\
                  \"success_criteria_check\": \"what has been verified\",\n\
                  \"actions\": [\n\
                    {{ \"action\": \"key\", \"keys\": [\"Command\", \"Space\"] }},\n\
                    {{ \"action\": \"type\", \"text\": \"safari\" }},\n\
                    {{ \"action\": \"wait\", \"seconds\": 2 }}\n\
                  ]\n\
                }}\n\n\
                {}\n\n\
                RULES:\n\
                1. If you need more detail, use {{ \"action\": \"inspect\" }}.\n\
                2. If you need to see the screen, use {{ \"action\": \"screenshot\" }}.\n\
                3. UI Tree Format: - [Role] \"Title\" (Description) value=\"Value\" @ [x, y, width, height].\n\
                4. If task is done, set \"completed\": true and return empty \"actions\".\n\
                5. Output valid JSON only.",
                DesktopAction::description()
            )
        } else {
            format!(
                "You are an expert computer control agent. You receive screenshots and user instructions.\n\n\
                GOAL: Complete the user's task by outputting a JSON object with 'thoughts' and 'actions'.\n\n\
                RESPONSE FORMAT:\n\
                {{\n\
                  \"thoughts\": \"I see the desktop. I will open Safari...\",\n\
                  \"completed\": false,\n\
                  \"done_reason\": \"why still running or why done\",\n\
                  \"success_criteria_check\": \"what has been verified\",\n\
                  \"actions\": [\n\
                    {{ \"action\": \"key\", \"keys\": [\"Command\", \"Space\"] }},\n\
                    {{ \"action\": \"screenshot\" }}\n\
                  ]\n\
                }}\n\n\
                {}\n\n\
                RULES:\n\
                1. You will receive a screenshot on the first step. For subsequent steps, you will only receive the UI tree unless you explicitly call {{ \"action\": \"screenshot\" }}.\n\
                2. Use {{ \"action\": \"screenshot\" }} whenever you need to verify the visual state.\n\
                3. Use {{ \"action\": \"inspect\" }} for a detailed UI tree.\n\
                4. If task is done, set \"completed\": true and return empty \"actions\".\n\
                5. Output valid JSON only.",
                DesktopAction::description()
            )
        };

        messages.push(json!({ "role": "system", "content": system_content }));

        let mut last_action_results = Vec::new();
        const MAX_NON_SYSTEM_MESSAGES: usize = 12;
        let high_risk_approved = has_high_risk_approval(&req.instruction);
        let mut completion_flag = false;
        let mut completion_reason = String::new();
        let mut completion_check = String::new();

        for step in 0..max_steps {
            if step > 0 {
                yield Ok(Event::default().event("step").data(format!("Step {}/{}", step + 1, max_steps)));
            }

            // 2. Capture State (Screenshot AND/OR UI Tree)
            let mut b64_image = None;
            let mut ui_tree = None;
            let need_screenshot = !is_text_only_model && (step == 0 || capture_screenshot);
            let mut need_ui_tree = is_text_only_model || !need_screenshot;

            if need_screenshot {
                match capture_screenshot_with_timeout().await {
                    Ok(img) => b64_image = Some(img),
                    Err(e) => {
                        need_ui_tree = true;
                        yield Ok(Event::default().data(format!("Error: Failed to capture screen: {}", e)));
                    }
                }
            }

            if need_ui_tree {
                if detailed_mode {
                    yield Ok(
                        Event::default()
                            .event("step")
                            .data("Inspect requested, capturing lightweight UI snapshot"),
                    );
                }
                match capture_ui_tree_with_timeout(detailed_mode).await {
                    Ok(tree) => ui_tree = Some(tree),
                    Err(e) => ui_tree = Some(format!("Error capturing UI tree: {}", e)),
                }
            }

            // Reset flags after capture
            detailed_mode = false;
            capture_screenshot = false;

            // 3. Construct User Message
            let feedback = if last_action_results.is_empty() {
                String::new()
            } else {
                format!("\nPREVIOUS ACTION RESULTS:\n- {}\n", last_action_results.join("\n- "))
            };

            let user_msg = if step == 0 {
                if let Some(img) = b64_image {
                    json!({
                        "role": "user",
                        "content": [
                            { "type": "text", "text": format!("Instruction: {}\n{}", req.instruction, feedback) },
                            { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{}", img) } }
                        ]
                    })
                } else if let Some(tree) = ui_tree {
                     json!({
                        "role": "user",
                        "content": format!("Instruction: {}\n{}\nCurrent UI State:\n{}", req.instruction, feedback, tree)
                    })
                } else {
                    json!({ "role": "user", "content": format!("Instruction: {}\n{}", req.instruction, feedback) })
                }
            } else {
                // Follow-up steps
                if let Some(img) = b64_image {
                    json!({
                        "role": "user",
                        "content": [
                            { "type": "text", "text": format!("Action executed. New screen state below. Proceed.\n{}", feedback) },
                            { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{}", img) } }
                        ]
                    })
                } else if let Some(tree) = ui_tree {
                     json!({
                        "role": "user",
                        "content": format!("Action executed. New UI State:\n{}\n{}\nProceed.", tree, feedback)
                    })
                } else {
                    json!({ "role": "user", "content": format!("Action executed. Proceed.\n{}", feedback) })
                }
            };

            messages.push(user_msg);
            last_action_results.clear();

            // 4. Call AI
            let inference_messages = build_inference_messages(&messages, MAX_NON_SYSTEM_MESSAGES);
            let ai_stream_result = if is_text_only_model {
                state.client.chat_stream(inference_messages).await
            } else {
                state
                    .client
                    .chat_stream_with_model(inference_messages, Some(vision_model.as_str()))
                    .await
            };
            let mut ai_stream = match ai_stream_result {
                Ok(s) => s,
                Err(e) => {
                    yield Ok(Event::default().data(format!("Error: AI Error: {}", e)));
                    return;
                }
            };

            let mut full_response = String::new();
            while let Some(chunk_result) = ai_stream.next().await {
                match chunk_result {
                    Ok(chunks) => {
                        let mut content_batch = String::new();
                        for chunk in chunks {
                            match chunk {
                                StreamChunk::Content(text) => {
                                    full_response.push_str(&text);
                                    content_batch.push_str(&text);
                                }
                                StreamChunk::Reasoning(text) => {
                                    yield Ok(Event::default().event("reasoning").data(text));
                                }
                            }
                        }
                        if !content_batch.is_empty() {
                            yield Ok(Event::default().data(content_batch));
                        }
                    }
                    Err(e) => {
                        yield Ok(Event::default().data(format!("Error: Stream Error: {}", e)));
                    }
                }
            }

            // 5. Parse & Execute Actions
            info!("AI Step {} Response: {}", step, full_response);
            messages.push(json!({ "role": "assistant", "content": full_response }));
            let (completed, done_reason, success_check) = parse_completion_meta(&full_response);
            if let Some(v) = completed {
                completion_flag = v;
            }
            if let Some(v) = done_reason {
                completion_reason = v;
            }
            if let Some(v) = success_check {
                completion_check = v;
            }

            let actions = parse_actions(&full_response);

            if actions.is_empty() {
                info!("No actions parsed in step {}. Assuming done.", step);
                if step > 0
                    || !full_response.trim().is_empty()
                    || full_response.to_lowercase().contains("done")
                    || full_response.to_lowercase().contains("completed")
                {
                    break;
                }
            }

            for action in actions {
                let act_type = action["action"]
                    .as_str()
                    .or_else(|| action["type"].as_str())
                    .unwrap_or("unknown");

                if act_type == "inspect" {
                    detailed_mode = true;
                }
                if act_type == "screenshot" {
                    capture_screenshot = true;
                }
                if is_high_risk_action(&action) && !high_risk_approved {
                    let blocked = build_high_risk_block_message(&action);
                    last_action_results.push(format!("FAILED: {}", blocked));
                    yield Ok(Event::default().event("error").data(blocked));
                    continue;
                }

                match execute_action_value(&state, &action).await {
                    Ok(desc) => {
                        last_action_results.push(format!("SUCCESS: {}", desc));
                        yield Ok(Event::default().event("action").data(desc));
                    }
                    Err(e) => {
                        last_action_results.push(format!("FAILED: {}", e));
                        yield Ok(Event::default().event("error").data(e));
                    }
                }
            }

            // Small pause between steps to let UI update
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        if completion_reason.is_empty() {
            completion_reason = "Task loop finished".to_string();
        }
        if completion_check.is_empty() {
            completion_check = "No explicit success check from model".to_string();
        }
        yield Ok(Event::default().event("done").data(json!({
            "completed": completion_flag,
            "done_reason": completion_reason,
            "success_criteria_check": completion_check
        }).to_string()));
    };

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

fn parse_actions(response: &str) -> Vec<Value> {
    if let Some(value) = parse_response_json(response)
        && let Some(actions) = parse_actions_from_value(value)
    {
        return actions;
    }

    vec![]
}

#[cfg(test)]
mod tests {
    use super::parse_actions;

    #[test]
    fn parse_actions_from_json_object() {
        let input = r#"{"thoughts":"ok","actions":[{"action":"key","keys":["Command","Space"]}]}"#;
        let actions = parse_actions(input);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["action"].as_str(), Some("key"));
    }

    #[test]
    fn parse_actions_from_fenced_json() {
        let input = r#"analysis
```json
{
  "thoughts": "go",
  "actions": [{"action":"type","text":"safari"}]
}
```
"#;
        let actions = parse_actions(input);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["action"].as_str(), Some("type"));
    }

    #[test]
    fn parse_actions_from_single_object() {
        let input = r#"{"action":"wait","seconds":1}"#;
        let actions = parse_actions(input);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["action"].as_str(), Some("wait"));
    }
}
