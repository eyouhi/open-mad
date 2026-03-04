use dioxus::prelude::*;
use futures::StreamExt;
use lucide_dioxus::*;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
enum MessageRole {
    User,
    AI,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct ChatMessage {
    id: String,
    role: MessageRole,
    content: String,
    actions: Vec<String>,
    is_streaming: bool,
    thoughts_collapsed: bool,
}

fn extract_thoughts(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(answer) = json
            .get("answer")
            .or_else(|| json.get("final_answer"))
            .or_else(|| json.get("message"))
            .or_else(|| json.get("response"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return answer.to_string();
        }

        if let Some(thoughts) = json
            .get("thoughts")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return thoughts.to_string();
        }

        if json.get("actions").is_some() {
            return "已收到任务，正在执行下方动作步骤。".to_string();
        }
    }

    if let Some(fence_start) = trimmed.find("```") {
        let fenced = &trimmed[fence_start + 3..];
        if let Some(fence_end) = fenced.find("```") {
            let candidate = fenced[..fence_end].trim_start_matches("json").trim();
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(candidate)
                && let Some(thoughts) = json
                    .get("thoughts")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            {
                return thoughts.to_string();
            }
        }
    }

    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return String::new();
    }

    trimmed.to_string()
}

fn decode_json_string_partial(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let mut hex = String::new();
                for _ in 0..4 {
                    let Some(next_char) = chars.peek().copied() else {
                        break;
                    };
                    if next_char.is_ascii_hexdigit() {
                        hex.push(next_char);
                        let _ = chars.next();
                    } else {
                        break;
                    }
                }
                if hex.len() == 4
                    && let Ok(code) = u32::from_str_radix(&hex, 16)
                    && let Some(decoded) = char::from_u32(code)
                {
                    out.push(decoded);
                }
            }
            Some(other) => out.push(other),
            None => break,
        }
    }
    out
}

fn extract_partial_thoughts_field(raw: &str) -> String {
    let Some(field_idx) = raw.find("\"thoughts\"") else {
        return String::new();
    };

    let after_field = &raw[field_idx + "\"thoughts\"".len()..];
    let Some(colon_idx) = after_field.find(':') else {
        return String::new();
    };
    let mut value_part = after_field[colon_idx + 1..].trim_start();
    if !value_part.starts_with('"') {
        return String::new();
    }

    value_part = &value_part[1..];
    let mut escaped = false;
    for (idx, ch) in value_part.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return decode_json_string_partial(&value_part[..idx]);
        }
    }

    decode_json_string_partial(value_part)
}

fn extract_thoughts_streaming(raw: &str) -> String {
    let full = extract_thoughts(raw);
    if !full.is_empty() {
        return full;
    }

    let trimmed = raw.trim();
    let partial = extract_partial_thoughts_field(trimmed);
    if !partial.is_empty() {
        return partial;
    }

    if let Some(fence_start) = trimmed.find("```") {
        let fenced = &trimmed[fence_start + 3..];
        let candidate = fenced.trim_start_matches("json").trim_start();
        let partial_in_fence = extract_partial_thoughts_field(candidate);
        if !partial_in_fence.is_empty() {
            return partial_in_fence;
        }
    }

    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return String::new();
    }

    trimmed.to_string()
}

fn progress_step_count(actions: &[String]) -> usize {
    actions
        .iter()
        .filter(|action| action.starts_with("➡️"))
        .count()
}

fn progress_percent(actions: &[String], is_streaming: bool) -> usize {
    if !is_streaming {
        return 100;
    }
    let steps = progress_step_count(actions);
    if steps == 0 {
        return 8;
    }
    (steps * 12 + 10).min(95)
}

fn classify_request_error(raw: &str) -> (String, String) {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("missing api key")
        || lower.contains("api key")
        || lower.contains("401")
        || lower.contains("unauthorized")
    {
        return (
            "后端已启动，但 AI 鉴权失败。".to_string(),
            "❌ 请在 ~/.open-mad/.env 或 ~/.open-mad/config.toml 配置 DEEPSEEK_API_KEY".to_string(),
        );
    }
    if lower.contains("connect")
        || lower.contains("no such file")
        || lower.contains("connection refused")
        || lower.contains("unix")
    {
        return (
            "无法连接本地后端服务。".to_string(),
            "❌ 请确认图标启动时后端已拉起，并检查 ~/.open-mad/mad.sock 是否可创建".to_string(),
        );
    }
    if lower.contains("blocked high-risk action") {
        return (
            "已触发高风险动作保护。".to_string(),
            "⚠️ 如确认执行，请在指令中加入 #confirm-risk 或“确认高风险操作”".to_string(),
        );
    }
    (raw.to_string(), "❌ 请查看错误详情并重试".to_string())
}

#[tokio::main]
async fn main() {
    // Setup panic hook to see what's happening
    std::panic::set_hook(Box::new(|info| {
        eprintln!("PANIC: {:?}", info);
    }));

    // Initialize server environment (logging, etc.)
    mad_server::config::init_env();

    // Different logging levels for different build profiles
    #[cfg(debug_assertions)]
    let max_level = tracing::Level::DEBUG;
    #[cfg(not(debug_assertions))]
    let max_level = tracing::Level::WARN;

    tracing_subscriber::fmt().with_max_level(max_level).init();

    println!("Starting MAD Server from UI (println)...");
    tracing::info!("Starting MAD Server from UI...");
    // Run the server in a separate task
    tokio::spawn(async {
        use futures::FutureExt;
        let result = std::panic::AssertUnwindSafe(mad_server::run_server())
            .catch_unwind()
            .await;

        match result {
            Ok(Ok(_)) => tracing::info!("MAD Server exited normally"),
            Ok(Err(e)) => tracing::error!("MAD Server exited with error: {:?}", e),
            Err(e) => tracing::error!("MAD Server panicked: {:?}", e),
        }
    });

    // Launch Dioxus on the main thread
    // This blocks the main thread, which is required on macOS
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Stylesheet { href: asset!("/assets/style.css") }
        document::Title { "OPEN MAD" }
        main { class: "h-screen flex flex-col bg-slate-50 overflow-hidden font-sans selection:bg-indigo-100 selection:text-indigo-900 text-slate-900",
            // Navbar
            header { class: "bg-white/80 backdrop-blur-md border-b border-slate-200/60 px-8 py-4 flex items-center justify-between shadow-sm z-10",
                div { class: "flex items-center gap-3",
                    div { class: "bg-gradient-to-br from-indigo-600 to-purple-600 text-white p-2.5 rounded-xl shadow-indigo-100 shadow-lg",
                        Bot { size: 22, stroke_width: 2 }
                    }
                    div { class: "flex flex-col",
                        h1 { class: "text-lg font-black text-slate-800 tracking-tight leading-none italic",
                            "OPEN MAD"
                        }
                        span { class: "text-[10px] text-slate-400 font-bold uppercase tracking-widest mt-1",
                            "Multi-Agent Desktop"
                        }
                    }
                }
                div { class: "flex items-center gap-4",
                    div { class: "flex items-center gap-1.5 px-3 py-1.5 bg-slate-100 rounded-full border border-slate-200/50",
                        div { class: "w-2 h-2 bg-emerald-500 rounded-full animate-pulse" }
                        span { class: "text-[11px] font-bold text-slate-500 uppercase tracking-wider",
                            "Connected"
                        }
                    }
                    span { class: "text-[11px] font-black bg-indigo-50 text-indigo-600 px-2.5 py-1.5 rounded-lg border border-indigo-100",
                        "v0.1.0"
                    }
                }
            }

            // Content
            div { class: "flex-1 overflow-hidden relative", ChatPage {} }
        }
    }
}

#[component]
fn ChatPage() -> Element {
    let mut messages = use_signal(Vec::<ChatMessage>::new);
    let mut input = use_signal(String::new);
    let mut is_loading = use_signal(|| false);
    let mut risk_confirm_instruction = use_signal(|| Option::<String>::None);
    let mut risk_confirm_reason = use_signal(String::new);
    let mut approve_next_high_risk = use_signal(|| false);

    use_effect(move || {
        let snapshot = messages.read();
        let _message_change_token: usize = snapshot
            .iter()
            .map(|msg| msg.content.len() + msg.actions.len())
            .sum();
        let _ = document::eval(
            r#"
            const el = document.getElementById("chat-scroll-container");
            if (el) {
                el.scrollTop = el.scrollHeight;
            }
            "#,
        );
    });

    let socket_path = use_memo(|| {
        let config = mad_server::config::load_config();
        std::env::var("MAD_SOCKET_PATH")
            .ok()
            .or_else(|| config.get_socket_path())
            .unwrap_or_else(mad_server::config::default_socket_path)
    });

    let mut send_message = move || {
        let text = input.read().clone();
        if text.trim().is_empty() || *is_loading.read() {
            return;
        }
        let approved = *approve_next_high_risk.read();
        if approved {
            approve_next_high_risk.set(false);
        }
        let request_instruction = if approved {
            format!("{}\n#confirm-risk", text)
        } else {
            text.clone()
        };

        let user_msg = ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: MessageRole::User,
            content: text.clone(),
            actions: Vec::new(),
            is_streaming: false,
            thoughts_collapsed: false,
        };

        messages.write().push(user_msg);
        input.set(String::new());
        is_loading.set(true);

        let ai_msg_id = Uuid::new_v4().to_string();
        let ai_msg = ChatMessage {
            id: ai_msg_id.clone(),
            role: MessageRole::AI,
            content: String::new(),
            actions: Vec::new(),
            is_streaming: true,
            thoughts_collapsed: false,
        };
        let ai_msg_idx = {
            let mut msgs = messages.write();
            msgs.push(ai_msg);
            msgs.len() - 1
        };

        let socket_path = socket_path.read().clone();
        let api_url = "http://localhost/api/chat/stream".to_string();

        spawn(async move {
            let client = match reqwest::Client::builder().unix_socket(socket_path).build() {
                Ok(client) => client,
                Err(e) => {
                    tracing::error!("Failed to create unix socket client: {:?}", e);
                    is_loading.set(false);
                    let mut msgs = messages.write();
                    if let Some(msg) = msgs.get_mut(ai_msg_idx) {
                        msg.content = format!("无法创建本地连接：{}", e);
                        msg.actions.push(
                            "❌ 后端未就绪，请检查 ~/.open-mad/.env 或 ~/.open-mad/config.toml 的 API Key 配置"
                                .to_string(),
                        );
                        msg.is_streaming = false;
                        msg.thoughts_collapsed = false;
                    }
                    return;
                }
            };
            let res = client
                .post(&api_url)
                .json(&serde_json::json!({ "instruction": request_instruction }))
                .send()
                .await;
            let mut final_content = String::new();
            let mut request_error: Option<String> = None;
            let mut high_risk_blocked = false;
            let mut high_risk_reason = String::new();

            match res {
                Ok(response) => {
                    if !response.status().is_success() {
                        let status = response.status();
                        let body = response
                            .text()
                            .await
                            .unwrap_or_else(|_| "读取错误详情失败".to_string());
                        request_error = Some(format!("请求失败（{}）：{}", status, body));
                    } else {
                        let mut stream = response.bytes_stream();
                        let mut current_content = String::new();
                        let mut pending_actions: Vec<String> = Vec::new();
                        let mut sse_buffer = String::new();
                        let mut done = false;
                        let mut last_flush = Instant::now();
                        let flush_interval = Duration::from_millis(80);
                        let mut content_since_flush = 0usize;

                        while !done {
                            let Some(chunk) = stream.next().await else {
                                break;
                            };

                            if let Ok(bytes) = chunk {
                                sse_buffer.push_str(&String::from_utf8_lossy(&bytes));

                                while let Some(frame_end) = sse_buffer.find("\n\n") {
                                    let frame = sse_buffer[..frame_end].to_string();
                                    sse_buffer.drain(..frame_end + 2);

                                    let mut event_name = String::new();
                                    let mut data_lines = Vec::new();
                                    for raw_line in frame.lines() {
                                        let line = raw_line.trim_end_matches('\r');
                                        if let Some(v) = line.strip_prefix("event:") {
                                            event_name = v.trim().to_string();
                                        } else if let Some(v) = line.strip_prefix("data:") {
                                            data_lines.push(v.trim_start().to_string());
                                        }
                                    }

                                    if data_lines.is_empty() {
                                        continue;
                                    }

                                    let data = data_lines.join("\n");
                                    if data == "[DONE]" {
                                        done = true;
                                        break;
                                    }
                                    if event_name == "done" {
                                        if let Ok(done_meta) =
                                            serde_json::from_str::<serde_json::Value>(&data)
                                        {
                                            let completed = done_meta
                                                .get("completed")
                                                .and_then(|v| v.as_bool())
                                                .unwrap_or(false);
                                            let done_reason = done_meta
                                                .get("done_reason")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("未提供");
                                            let check = done_meta
                                                .get("success_criteria_check")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("未提供");
                                            pending_actions.push(format!(
                                                "🏁 完成判定：{} | 原因：{} | 校验：{}",
                                                if completed { "通过" } else { "未通过" },
                                                done_reason,
                                                check
                                            ));
                                        } else {
                                            pending_actions.push(format!("🏁 {}", data));
                                        }
                                        done = true;
                                        break;
                                    }

                                    if event_name == "action" {
                                        pending_actions.push(format!("✅ {}", data));
                                    } else if event_name == "error" {
                                        if data
                                            .to_ascii_lowercase()
                                            .contains("blocked high-risk action")
                                        {
                                            high_risk_blocked = true;
                                            high_risk_reason = data.clone();
                                            pending_actions.push(format!("⚠️ {}", data));
                                            done = true;
                                            break;
                                        }
                                        pending_actions.push(format!("❌ {}", data));
                                    } else if event_name == "step" {
                                        pending_actions.push(format!("➡️ {}", data));
                                    } else if event_name == "content"
                                        || event_name.is_empty()
                                        || event_name == "reasoning"
                                    {
                                        current_content.push_str(&data);
                                        content_since_flush += data.len();
                                    }

                                    let should_flush = content_since_flush > 0
                                        || !pending_actions.is_empty()
                                        || last_flush.elapsed() >= flush_interval
                                        || done;

                                    if should_flush {
                                        let mut msgs = messages.write();
                                        if let Some(msg) = msgs.get_mut(ai_msg_idx) {
                                            msg.content.clear();
                                            msg.content.push_str(&extract_thoughts_streaming(
                                                &current_content,
                                            ));
                                            if !pending_actions.is_empty() {
                                                msg.actions.append(&mut pending_actions);
                                            }
                                        }
                                        last_flush = Instant::now();
                                        content_since_flush = 0;
                                    }
                                }
                            }
                        }
                        final_content = current_content;
                    }
                }
                Err(e) => {
                    tracing::error!("Fetch error: {:?}", e);
                    request_error = Some(format!("无法连接后端服务：{}", e));
                }
            }
            is_loading.set(false);

            let mut msgs = messages.write();
            if let Some(msg) = msgs.get_mut(ai_msg_idx) {
                if high_risk_blocked {
                    msg.content = "检测到高风险操作，请确认后继续执行。".to_string();
                    msg.actions.push(format!("⚠️ {}", high_risk_reason));
                    msg.actions
                        .push("🔐 点击下方确认框中的“确认执行”继续".to_string());
                    msg.thoughts_collapsed = false;
                    risk_confirm_instruction.set(Some(text.clone()));
                    risk_confirm_reason.set(high_risk_reason);
                } else if let Some(err) = request_error {
                    let (friendly, hint) = classify_request_error(&err);
                    msg.content = friendly;
                    msg.actions.push(hint);
                    msg.actions.push(format!("ℹ️ 详情：{}", err));
                    msg.thoughts_collapsed = false;
                } else {
                    let final_thoughts = extract_thoughts_streaming(&final_content);
                    if !final_thoughts.is_empty() {
                        msg.content = final_thoughts;
                    }
                    msg.thoughts_collapsed = true;
                }
                msg.is_streaming = false;
            }
        });
    };

    rsx! {
        div { class: "h-full flex flex-col w-full bg-slate-50/50 backdrop-blur-xl",
            // Messages area
            div { id: "chat-scroll-container", class: "flex-1 overflow-y-auto pt-10 pb-4 scroll-smooth",
                div { class: "max-w-3xl mx-auto w-full px-4 space-y-8",
                    for msg in messages.read().iter().cloned() {
                        div {
                            key: "{msg.id}",
                            class: if msg.role == MessageRole::AI { "group flex animate-in fade-in slide-in-from-bottom-2 duration-300 gap-4 items-start" } else { "group flex animate-in fade-in slide-in-from-bottom-2 duration-300 gap-4 flex-row-reverse items-start" },

                            // Avatar
                            div { class: if msg.role == MessageRole::AI { "w-9 h-9 rounded-xl flex items-center justify-center shrink-0 shadow-sm transition-transform group-hover:scale-105 bg-gradient-to-br from-emerald-400 to-emerald-600 text-white" } else { "w-9 h-9 rounded-xl flex items-center justify-center shrink-0 shadow-sm transition-transform group-hover:scale-105 bg-gradient-to-br from-indigo-500 to-purple-600 text-white" },
                                if msg.role == MessageRole::AI {
                                    Bot { size: 20 }
                                } else {
                                    User { size: 20 }
                                }
                            }

                            div { class: if msg.role == MessageRole::AI { "flex flex-col max-w-[85%] gap-2 items-start" } else { "flex flex-col max-w-[85%] gap-2 items-end" },
                                // Content bubble
                                div { class: if msg.role == MessageRole::AI { "relative px-5 py-3.5 shadow-sm text-[15px] leading-relaxed transition-all bg-white border border-slate-200/60 text-slate-800 rounded-2xl rounded-tl-none hover:shadow-md" } else { "relative px-5 py-3.5 shadow-sm text-[15px] leading-relaxed transition-all bg-gradient-to-r from-indigo-600 to-purple-600 text-white rounded-2xl rounded-tr-none hover:shadow-md" },
                                    if msg.role == MessageRole::AI {
                                        button {
                                            class: "w-full flex items-center justify-between text-[11px] uppercase tracking-wider font-bold text-slate-500 mb-2",
                                            onclick: move |_| {
                                                let mut msgs = messages.write();
                                                if let Some(target) = msgs.iter_mut().find(|m| m.id == msg.id) {
                                                    target.thoughts_collapsed = !target.thoughts_collapsed;
                                                }
                                            },
                                            span { "Thoughts" }
                                            if msg.thoughts_collapsed && !msg.is_streaming {
                                                ChevronDown { size: 14 }
                                            } else {
                                                ChevronUp { size: 14 }
                                            }
                                        }
                                    }
                                    if msg.role != MessageRole::AI || !msg.thoughts_collapsed || msg.is_streaming {
                                        "{msg.content}"
                                    } else {
                                        p { class: "text-[13px] text-slate-400 italic",
                                            "已折叠"
                                        }
                                    }
                                    if msg.is_streaming && msg.content.is_empty() {
                                        div { class: "flex gap-1 items-center py-1",
                                            div { class: "w-1.5 h-1.5 bg-slate-300 rounded-full animate-bounce [animation-delay:-0.3s]" }
                                            div { class: "w-1.5 h-1.5 bg-slate-300 rounded-full animate-bounce [animation-delay:-0.15s]" }
                                            div { class: "w-1.5 h-1.5 bg-slate-300 rounded-full animate-bounce" }
                                        }
                                    }
                                }

                                if msg.role == MessageRole::AI && !msg.actions.is_empty() {
                                    div { class: "w-full mt-1 animate-in fade-in slide-in-from-top-1 duration-500",
                                        div { class: "bg-slate-100/80 backdrop-blur-sm border border-slate-200/50 rounded-xl p-3 text-[13px] text-slate-700 overflow-x-auto whitespace-pre-wrap leading-snug",
                                            if msg.actions.iter().any(|action| action.starts_with("➡️")) {
                                                div { class: "mb-3 pb-3 border-b border-slate-200/50",
                                                    div { class: "flex items-center justify-between text-[10px] font-bold text-slate-500 uppercase tracking-tighter mb-2",
                                                        span { class: "flex items-center gap-2",
                                                            Activity { size: 12 }
                                                            "Progress"
                                                        }
                                                        span { class: "font-medium text-slate-400 normal-case tracking-normal",
                                                            "{progress_percent(&msg.actions, msg.is_streaming)}% · {progress_step_count(&msg.actions)} steps"
                                                        }
                                                    }
                                                    progress {
                                                        class: if msg.is_streaming { "w-full h-2 accent-indigo-500 animate-pulse" } else { "w-full h-2 accent-indigo-500" },
                                                        max: "100",
                                                        value: "{progress_percent(&msg.actions, msg.is_streaming)}",
                                                    }
                                                }
                                            }
                                            if msg.actions.iter().any(|action| !action.starts_with("➡️")) {
                                                div { class: "flex items-center gap-2 mb-2 pb-2 border-b border-slate-200/50 font-bold text-slate-500 uppercase tracking-tighter text-[10px]",
                                                    Command { size: 12 }
                                                    "Action Steps"
                                                }
                                            }
                                            for (idx , action) in msg.actions.iter().filter(|action| !action.starts_with("➡️")).enumerate() {
                                                p { class: "py-0.5", "{idx + 1}. {action}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Input area
            div { class: "pb-8 pt-4 bg-white/40 backdrop-blur-md border-t border-slate-200/60",
                div { class: "max-w-3xl mx-auto w-full px-4",
                    div { class: "relative group flex items-end gap-2 bg-white border border-slate-200 rounded-2xl p-2 pr-3 shadow-lg focus-within:ring-2 focus-within:ring-indigo-500/10 focus-within:border-indigo-500/50 transition-all",
                        textarea {
                            rows: "1",
                            placeholder: "Ask me to do something on your computer...",
                            class: "flex-1 bg-transparent border-none focus:ring-0 resize-none py-3 pl-3 text-[15px] max-h-48 scrollbar-hide",
                            value: "{input}",
                            oninput: move |ev| input.set(ev.value()),
                            onkeydown: move |ev| {
                                if ev.key() == Key::Enter && !ev.modifiers().shift() {
                                    send_message();
                                }
                            },
                        }
                        button {
                            class: "mb-1 p-2.5 bg-indigo-600 text-white rounded-xl hover:bg-indigo-700 transition-all shadow-indigo-200 shadow-lg disabled:opacity-40 disabled:shadow-none disabled:cursor-not-allowed hover:scale-105 active:scale-95",
                            disabled: *is_loading.read() || input.read().trim().is_empty(),
                            onclick: move |_| send_message(),
                            if *is_loading.read() {
                                div { class: "animate-spin",
                                    Loader { size: 18, stroke_width: 3 }
                                }
                            } else {
                                Send { size: 18, stroke_width: 2 }
                            }
                        }
                    }
                    div { class: "flex items-center justify-between mt-3 px-2",
                        p { class: "text-[10px] text-slate-400 uppercase tracking-[0.2em] font-bold",
                            "System Controlled by Open MAD"
                        }
                        div { class: "flex gap-2 text-[10px] text-slate-400 font-medium italic",
                            "Press Enter to send"
                        }
                    }
                }
            }
            if let Some(pending_instruction) = risk_confirm_instruction.read().clone() {
                div { class: "absolute inset-0 z-50 bg-slate-900/30 backdrop-blur-[1px] flex items-center justify-center px-4",
                    div { class: "w-full max-w-lg rounded-2xl bg-white border border-slate-200 shadow-2xl p-5 space-y-4",
                        div { class: "flex items-center gap-2 text-amber-600",
                            ShieldAlert { size: 18 }
                            h3 { class: "text-sm font-bold tracking-wide uppercase",
                                "高风险操作确认"
                            }
                        }
                        p { class: "text-sm text-slate-700 leading-relaxed",
                            "{risk_confirm_reason.read()}"
                        }
                        p { class: "text-xs text-slate-500 leading-relaxed",
                            "继续后将按当前指令执行潜在高风险动作。"
                        }
                        div { class: "grid grid-cols-2 gap-2 pt-1",
                            button {
                                class: "w-full px-3 py-2 rounded-lg border border-slate-300 text-slate-600 hover:bg-slate-50 transition-colors",
                                onclick: move |_| {
                                    risk_confirm_instruction.set(None);
                                    risk_confirm_reason.set(String::new());
                                },
                                "取消"
                            }
                            button {
                                class: "w-full px-3 py-2 rounded-lg bg-indigo-600 text-white hover:bg-indigo-700 transition-colors shadow-sm border border-indigo-700/40 font-semibold",
                                onclick: move |_| {
                                    risk_confirm_instruction.set(None);
                                    risk_confirm_reason.set(String::new());
                                    approve_next_high_risk.set(true);
                                    input.set(pending_instruction.clone());
                                    send_message();
                                },
                                "确认执行高风险操作"
                            }
                        }
                    }
                }
            }
        }
    }
}
