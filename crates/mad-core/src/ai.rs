use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::pin::Pin;
use tracing::info;

pub struct DeepseekClient {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub enum StreamChunk {
    Content(String),
    Reasoning(String),
}

impl DeepseekClient {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.deepseek.com".to_string()),
            model: model.unwrap_or_else(|| "deepseek-chat".to_string()),
        }
    }

    pub async fn chat(&self, messages: Vec<serde_json::Value>) -> Result<String> {
        self.chat_with_model(messages, None).await
    }

    pub async fn chat_with_model(
        &self,
        messages: Vec<serde_json::Value>,
        model: Option<&str>,
    ) -> Result<String> {
        let url = format!("{}/chat/completions", self.base_url);

        let payload = json!({
            "model": model.unwrap_or(&self.model),
            "messages": messages,
            "stream": false
        });

        info!("Sending request to Deepseek API: {}", url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to Deepseek API")?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("API Error: {}", error_text);
        }

        let json: serde_json::Value = response.json().await?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .context("No content in response")?
            .to_string();

        Ok(content)
    }

    pub async fn chat_stream(
        &self,
        messages: Vec<serde_json::Value>,
    ) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<Vec<StreamChunk>>> + Send>>> {
        self.chat_stream_with_model(messages, None).await
    }

    pub async fn chat_stream_with_model(
        &self,
        messages: Vec<serde_json::Value>,
        model: Option<&str>,
    ) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<Vec<StreamChunk>>> + Send>>> {
        let url = format!("{}/chat/completions", self.base_url);

        let payload = json!({
            "model": model.unwrap_or(&self.model),
            "messages": messages,
            "stream": true
        });

        info!("Sending streaming request to Deepseek API: {}", url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to Deepseek API")?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("API Error: {}", error_text);
        }

        let stream = response
            .bytes_stream()
            .scan(String::new(), |sse_buffer, chunk_result| {
                let bytes = match chunk_result {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        return futures_util::future::ready(Some(Err(
                            anyhow::Error::new(e).context("Failed to read chunk")
                        )));
                    }
                };

                sse_buffer.push_str(&String::from_utf8_lossy(&bytes));
                let mut parsed_chunks = Vec::new();

                while let Some(frame_end) = sse_buffer.find("\n\n") {
                    let frame = sse_buffer[..frame_end].to_string();
                    sse_buffer.drain(..frame_end + 2);

                    let mut data_lines = Vec::new();
                    for raw_line in frame.lines() {
                        let line = raw_line.trim_end_matches('\r');
                        if let Some(v) = line.strip_prefix("data:") {
                            data_lines.push(v.trim_start());
                        }
                    }

                    if data_lines.is_empty() {
                        continue;
                    }

                    let payload = data_lines.join("\n");
                    if payload == "[DONE]" {
                        continue;
                    }

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&payload) {
                        if let Some(reasoning) = json["choices"][0]["delta"]["reasoning_content"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                        {
                            parsed_chunks.push(StreamChunk::Reasoning(reasoning.to_string()));
                        }
                        if let Some(content) = json["choices"][0]["delta"]["content"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                        {
                            parsed_chunks.push(StreamChunk::Content(content.to_string()));
                        }
                    }
                }

                futures_util::future::ready(Some(Ok(parsed_chunks)))
            })
            .filter_map(|result| async move {
                match result {
                    Ok(chunks) if chunks.is_empty() => None,
                    _ => Some(result),
                }
            });

        Ok(Box::pin(stream))
    }
}
