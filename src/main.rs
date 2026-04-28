mod agent;
mod config;
mod kraken_futures;
mod kraken_types;
mod llm;
mod logging;
mod market_regimes;
mod polymarket;
mod ta;
mod tool;
mod trade;
mod utils;
mod workflow;

use crate::config::Config;
use crate::kraken_futures::KrakenFuturesRestClient;
use crate::kraken_types::KrakenEnvironment;
use crate::llm::LlmClient;
use crate::logging::init_logging;
use crate::workflow::Workflow;

#[tokio::main]
async fn main() {
    let _guard = init_logging("logs");

    tracing::info!(
        event = "startup",
        version = env!("CARGO_PKG_VERSION"),
        "Simple Trading Agent starting up"
    );

    let config = Config::load().expect("No config found");

    let provider: LlmClient = LlmClient::new(
        &config.llm_base_url,
        config.llm_api_key.clone(),
        &config.llm_model_name,
    )
    .expect("Failed to create LlmClient");

    let kraken_client = KrakenFuturesRestClient::with_auth(
        KrakenEnvironment::Demo,
        config.kraken_futures_api_key.clone(),
        config.kraken_futures_api_secret.clone(),
    );

    let mut workflow: Workflow = Workflow::new("PF_XBTUSD", provider, kraken_client);

    workflow
        .start_workflow()
        .await
        .expect("Failed to start workflow...");
}
