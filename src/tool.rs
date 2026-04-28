use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

#[async_trait(?Send)]
pub trait Tool: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    async fn execute(&self, args: Value) -> Result<Value, Box<dyn Error>>;
}

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T: Tool>(&mut self, tool: T) -> &mut Self {
        self.tools.insert(tool.name().to_string(), Arc::new(tool));
        self
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.values().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.len() == 0
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }
}

/// =============================================================================
/// FINAL ANSWER TOOL - STRUCTURED OUTPUT WORKAROUND
/// =============================================================================
///
/// PROBLEM:
/// Many LLM providers become unreliable or completely broken when you send BOTH:
///   - `tools` + `tool_choice`
///   - AND `response_format: { "type": "json_schema", ... }`
///
/// in the same API request.
///
/// The model receives two conflicting output instructions at the same time:
/// 1. "You can call tools when needed"
/// 2. "When you're done, you must return perfect JSON matching this schema"
///
/// → The model gets confused and usually follows only one of the two rules.
///    Either it ignores tools, or it uses tools but returns unstructured text/markdown.
///
/// SOLUTION:
/// We completely remove `response_format` / json_schema from the API call.
/// Instead, we turn the desired structured output into a normal tool called `final_answer`.
///
/// Why this works:
/// - The model now only has **one** output format: tool calls.
/// - The `final_answer` tool's `parameters` schema is defined to be exactly the JSON
///   structure we want.
/// - In `Agent::run()`, we specially detect the `final_answer` tool call and extract
///   its `arguments` directly — this becomes our clean structured output.
///
/// Benefits:
/// - Works reliably across almost all modern models (Claude, Gemini, GPT, Qwen, etc.)
/// - No more mixing of schemas
/// - The model is forced to output valid JSON (enforced by the tool schema)
/// - Much easier to debug (everything is visible as tool calls)
///
/// See also: `Agent::run()` (special handling for "final_answer") and `create_regime_agent()`.
/// =============================================================================
#[derive(Clone)]
pub struct FinalAnswerTool {
    schema: Value,
}

impl FinalAnswerTool {
    pub fn new(schema: Value) -> Self {
        Self { schema }
    }
}

#[async_trait(?Send)]
impl Tool for FinalAnswerTool {
    fn name(&self) -> &str {
        "final_answer" // fixed name — this is the convention
    }

    fn description(&self) -> &str {
        "Call this tool ONLY when you are ready to give the final structured answer. \
         This is the ONLY way to output the final result. Do not output JSON or text directly."
    }

    fn parameters(&self) -> Value {
        self.schema.clone()
    }

    async fn execute(&self, args: Value) -> Result<Value, Box<dyn Error>> {
        // Just return the validated arguments (no execution needed)
        Ok(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct EchoTool {}
    struct TimeTool {}

    #[async_trait(?Send)]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echoes back the input arguments as JSON."
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                }
            })
        }

        async fn execute(&self, args: Value) -> Result<Value, Box<dyn Error>> {
            Ok(args) // just echo the input
        }
    }

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
    async fn test_get_nonexistent_tool_returns_none() {
        let registry = ToolRegistry::new();
        assert!(registry.get("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_execute_echo_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool {});

        let tool = registry.get("echo").expect("echo tool should exist");

        let input = serde_json::json!({"message": "hello world", "number": 42});
        let result = tool.execute(input.clone()).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), input);
    }

    #[tokio::test]
    async fn test_register_multiple_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool {}).register(TimeTool {});

        assert!(registry.tools().len() == 2);
    }
}
