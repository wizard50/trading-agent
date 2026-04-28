use crate::tool::Tool;
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::error::Error;
use std::sync::Arc;
use tokio::time::Duration;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct LlmClient {
    client: Client,
    pub base_url: String,
    api_key: SecretString,
    pub default_model: String,
}

impl LlmClient {
    pub fn new(
        base_url: &str,
        api_key: SecretString,
        default_model: &str,
    ) -> Result<Self, Box<dyn Error>> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(75)) // total request timeout (recommended 60-90s)
            .connect_timeout(Duration::from_secs(15)) // time to establish connection
            .build()
            .map_err(|e| format!("Failed to build reqwest client: {}", e))?;

        Ok(Self {
            client,
            base_url: base_url.to_string(),
            api_key,
            default_model: default_model.to_string(),
        })
    }

    pub async fn call(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[Arc<dyn Tool>],
        temperature: f64,
        structured_schema: Option<&Value>,
    ) -> Result<CompletionResponse, Box<dyn Error>> {
        let url = format!("{}/chat/completions", self.base_url);

        let mut request_body = json!({
            "model": model,
            "messages": messages,
            "temperature": temperature,
        });

        if !tools.is_empty() {
            request_body["tools"] = to_spec(tools);
            request_body["tool_choice"] = json!("auto");
        }

        if let Some(schema) = structured_schema {
            request_body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "crypto_regime_decision",
                    "strict": true,
                    "schema": schema
                }
            });
        }

        debug!(
            event = "llm_request",
            request_body = %request_body,
            model = model,
            "Sending request to LLM API"
        );

        let http_resp = self
            .client
            .post(url)
            .bearer_auth(self.api_key.expose_secret())
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !http_resp.status().is_success() {
            return Err(format!("HTTP {}: {}", http_resp.status(), http_resp.text().await?).into());
        }

        let api_resp = http_resp.json::<ApiResponse>().await?;
        let choice = api_resp
            .choices
            .first()
            .ok_or("No choices returned by model")?;

        Ok(CompletionResponse {
            message: choice.message.clone(),
            finish_reason: choice.finish_reason.clone(),
        })
    }
}

// helper
fn to_spec(tools: &[Arc<dyn Tool>]) -> Value {
    tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name":        tool.name(),
                    "description": tool.description(),
                    "parameters":  tool.parameters(),
                    "strict":      true
                }
            })
        })
        .collect()
}

#[derive(Debug)]
pub struct CompletionResponse {
    pub message: Message,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },

    User {
        content: String,
    },

    Assistant {
        content: Option<String>,
        #[serde(default)]
        tool_calls: Option<Vec<ToolCall>>,
    },

    Tool {
        content: String,
        tool_call_id: String, // must match the id from the ToolCall
    },
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub function: FunctionCall,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String, // JSON string
}
