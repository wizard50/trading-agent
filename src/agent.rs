use crate::llm::*;
use crate::tool::ToolRegistry;
use crate::utils::with_exponential_backoff;
use serde_json::Value;
use std::error::Error;
use tracing::debug;

#[derive(Clone)]
pub struct Agent {
    id: String,
    tool_registry: ToolRegistry,
    provider: LlmClient,
    history: Vec<Message>,
    system_prompt: String,
    schema: Option<Value>,
}

impl Agent {
    pub fn new(
        id: &str,
        system_prompt: &str,
        provider: LlmClient,
        tool_registry: ToolRegistry,
        schema: Option<Value>,
    ) -> Self {
        let history = vec![Message::System {
            content: system_prompt.to_string(),
        }];

        Self {
            id: id.to_string(),
            system_prompt: system_prompt.to_string(),
            tool_registry,
            provider,
            history,
            schema,
        }
    }

    pub async fn run(&mut self, user_message: &str) -> Result<String, Box<dyn Error>> {
        self.history.push(Message::User {
            content: user_message.to_string(),
        });

        let mut tool_call_count = 0;
        let max_tool_calls = self.tool_registry.len() * 3;

        loop {
            if tool_call_count >= max_tool_calls {
                return Err(format!(
                            "Agent exceeded maximum tool calls ({max_tool_calls}) — possible infinite loop prevented"
                        ).into());
            }

            let CompletionResponse {
                message,
                finish_reason,
            } = with_exponential_backoff(3, || async {
                self.provider
                    .call(
                        &self.provider.default_model,
                        &self.history,
                        &self.tool_registry.tools(),
                        0.7,
                        self.schema.as_ref(),
                    )
                    .await
            })
            .await?;

            self.history.push(message.clone());

            match message {
                Message::Assistant {
                    tool_calls: Some(tool_calls),
                    ..
                } if !tool_calls.is_empty() => {
                    tool_call_count += 1;

                    for tool_call in tool_calls {
                        // === SPECIAL HANDLING FOR STRUCTURED OUTPUT ===
                        // We treat `final_answer` as our structured output mechanism.
                        // This is the core of the workaround that avoids the tools + response_format conflict.
                        // We extract the arguments directly instead of executing the tool.
                        if tool_call.function.name == "final_answer" {
                            let args: Value =
                                serde_json::from_str(&tool_call.function.arguments)
                                    .map_err(|e| format!("Failed to parse final_answer: {}", e))?;

                            return Ok(serde_json::to_string(&args).map_err(|e| {
                                format!("Failed to serialize final answer: {}", e)
                            })?);
                        }

                        // === Normal tool execution (e.g. get_polymarket_sentiment) ===
                        let tool_result = self.execute_tool_call(&tool_call).await?;
                        debug!(
                            event = "tool_call_executed",
                            tool_name = %tool_call.function.name,
                            tool_id = %tool_call.id,
                            result = %tool_result,
                            "Tool executed successfully"
                        );

                        self.history.push(Message::Tool {
                            content: tool_result,
                            tool_call_id: tool_call.id,
                        });
                    }
                    continue;
                }
                Message::Assistant {
                    content: Some(content),
                    ..
                } => {
                    // final answer
                    return Ok(content);
                }
                _ => {
                    return Err("Model returned empty message".into());
                }
            }
        }
    }

    async fn execute_tool_call(&self, tool_call: &ToolCall) -> Result<String, Box<dyn Error>> {
        let tool = self
            .tool_registry
            .get(&tool_call.function.name) // you need to add this method to ToolRegistry
            .ok_or_else(|| format!("Tool not found: {}", tool_call.function.name))?;

        let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
            .map_err(|e| format!("Failed to parse tool arguments: {}", e))?;

        let result: Value = tool
            .execute(args)
            .await
            .map_err(|e| format!("Tool execution failed: {}", e))?;

        Ok(serde_json::to_string(&result)
            .map_err(|e| format!("Failed to serialize tool result: {}", e))?)
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tool::Tool;
    use async_trait::async_trait;
    use serde_json::{Value, json};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TimeTool {}

    #[async_trait(?Send)]
    impl Tool for TimeTool {
        fn name(&self) -> &str {
            "get_current_time"
        }

        fn description(&self) -> &str {
            "Get the current time. Call this whenever you need to know the current time, for example when a customer asks 'What time is it?'"
        }

        fn parameters(&self) -> serde_json::Value {
            Value::default()
        }

        async fn execute(&self, args: Value) -> Result<Value, Box<dyn Error>> {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            Ok(json!({"timestamp": timestamp}))
        }
    }

    #[tokio::test]
    async fn test_agent() {
        let config = Config::load().expect("No config found");
        let provider = LlmClient::new(
            &config.llm_base_url,
            config.llm_api_key.clone(),
            &config.llm_model_name,
        )
        .unwrap();

        let mut tools = ToolRegistry::new();
        tools.register(TimeTool {});

        let mut agent = Agent::new(
            "Time Agent",
            "You are a helpful assistant. Answer the user's question.",
            provider,
            tools,
            None,
        );

        agent
            .run("What time is it now?")
            .await
            .expect("Failed to run the agent");

        println!("TEST - history: {:?}", agent.history);
    }

    #[tokio::test]
    async fn test_agent_no_tools() {
        let config = Config::load().expect("No config found");
        let provider = LlmClient::new(
            &config.llm_base_url,
            config.llm_api_key.clone(),
            &config.llm_model_name,
        )
        .unwrap();

        let tools = ToolRegistry::new();

        let mut agent = Agent::new(
            "Time Agent",
            "You are a helpful assistant. Answer the user's question.",
            provider,
            tools,
            None,
        );

        let resp = agent
            .run("What time is it now?")
            .await
            .expect("Failed to run the agent");

        println!("{resp:?}");
    }
}
