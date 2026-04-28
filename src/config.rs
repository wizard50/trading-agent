use dotenvy::dotenv_override;
use secrecy::SecretString;
use std::env::{self};

#[derive(Debug, Clone)]
pub struct Config {
    pub kraken_futures_api_key: SecretString,
    pub kraken_futures_api_secret: SecretString,
    pub llm_api_key: SecretString,
    pub llm_base_url: String,
    pub llm_model_name: String,
}

impl Config {
    pub fn load() -> Result<Self, String> {
        // load .env if it exists, otherwise fall back to real environment variables
        dotenv_override().ok();

        Ok(Config {
            kraken_futures_api_key: SecretString::from(
                env::var("KRAKEN_FUTURES_API_KEY").map_err(|_| {
                    "KRAKEN_FUTURES_API_KEY is missing from .env / environment".to_string()
                })?,
            ),
            kraken_futures_api_secret: SecretString::from(
                env::var("KRAKEN_FUTURES_API_SECRET").map_err(|_| {
                    "KRAKEN_FUTURES_API_SECRET is missing from .env / environment".to_string()
                })?,
            ),
            llm_api_key: SecretString::from(
                env::var("LLM_API_KEY")
                    .map_err(|_| "LLM_API_KEY is missing from .env / environment".to_string())?,
            ),
            llm_base_url: String::from(
                env::var("LLM_BASE_URL")
                    .map_err(|_| "LLM_BASE_URL is missing from .env / environment".to_string())?,
            ),
            llm_model_name: String::from(
                env::var("LLM_MODEL_NAME")
                    .map_err(|_| "LLM_MODEL_NAME is missing from .env / environment".to_string())?,
            ),
        })
    }
}
