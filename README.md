# trading-agent

A minimal LLM-powered trading agent built from scratch to demonstrate core concepts of agent loops, tool use, and structured decision-making—without relying on heavy frameworks.

This is a **single, standalone agent** that makes independent trading decisions. It is intentionally simple to serve as a foundation before moving to multi-agent orchestration and highly autonomous systems.

## What This Is

This project is a **learning exercise** designed to show how AI agents actually work behind the scenes. Instead of using full agent frameworks, it implements everything manually:

- **Manual Agent Loop** – direct control over the LLM conversation cycle
- **Tool Registry** – dynamic tool registration and execution
- **Polymarket Sentiment Tool** – optional real-money crowd wisdom for regime confirmation
- **Workflow Orchestration** – clean state management (analysis → decision → execution)
- **Structured Logging** – complete trading journal with `tracing`

The agent trades on **Kraken Futures** (demo environment supported) using technical regime analysis and can optionally query **Polymarket** for real-money sentiment when signals are unclear.

## Features

- Real-time technical regime detection (trend, volatility, liquidity, momentum)
- LLM-powered decision making with structured output
- Optional Polymarket sentiment tool for crowd confirmation
- Automatic stop-loss + take-profit placement
- Full structured trading journal (JSON + pretty console output)
- Graceful error handling with exponential backoff
- Async-first Rust with Tokio

## Architecture

```
┌─────────────┐     ┌─────────────────┐     ┌─────────────────┐
│   Kraken    │────▶│  Market Regimes │────▶│   Regime Agent  │
│   Futures   │     │   (TA Analysis) │     │   (LLM Decision)│
└─────────────┘     └─────────────────┘     └────────┬────────┘
                                                     │
                         ┌─────────────────────────┘
                         ▼
                ┌─────────────────┐
                │   Tool Registry │◄─── Polymarket Sentiment Tool
                │                 │     (Real-money crowd wisdom)
                └─────────────────┘
                         │
                         ▼
                ┌─────────────────┐     ┌─────────────┐
                │  Trade Manager  │────▶│   Kraken    │
                │ (Order Execution)│    │   Futures   │
                └─────────────────┘     └─────────────┘
```

## Tools

### Polymarket Sentiment Tool (`get_polymarket_sentiment`)

Fetches real-time crowd sentiment from Polymarket prediction markets for BTC, ETH, and crypto-related events.

- **When used**: The LLM automatically calls this tool when technical regimes are unclear, ranging, weak, mixed, or contradictory.
- **Data source**: Top 10 highest-volume active markets from Polymarket’s Gamma API.
- **Returns**: Market questions with Yes-probability percentages and 24h volume.

**Example output:**
"Will Bitcoin hit $150k by June 30, 2026?" (1%) vol=$5.82M | "MegaETH market cap (FDV) >$2B one day after launch?" (26%) vol=$212K | "MegaETH market cap (FDV) >$800M one day after launch?" (94%) vol=$212K

## Prerequisites

- **Rust** (2024 edition)
- **LLM API Access** - Must support **tool use** (function calling).

## Setup

1. **Clone and enter the directory:**
```bash
git clone https://github.com/wizard50/trading-agent.git && cd trading-agent
```

2. **Create `.env` file:**
```env
# Kraken Futures API (use demo keys for testing)
KRAKEN_FUTURES_API_KEY=your_key_here
KRAKEN_FUTURES_API_SECRET=your_secret_here

# LLM Configuration (configure for your preferred provider)
LLM_API_KEY=your_openrouter_api_key
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_MODEL_NAME=x-ai/grok-4.1-fast
```

3. **Build:**
```bash
cargo build --release
```

## Running the Agent

```bash
cargo run --release
```

The agent runs in a continuous loop:

1. Fetches candle data (15m + 1h) and order book

2. Calculates technical regimes (trend, volatility, liquidity, momentum)

3. Sends summary to the LLM

4. LLM may call Polymarket tool if needed

5. Receives structured decision (BUY/SELL/HOLD + confidence + risk level)

6. Executes on Kraken Futures only if confidence is HIGH or VERY HIGH

7. Places stop-loss and take-profit orders

8. Logs everything to the trading journal

**Default cycle**: every **15 minutes**.

## Development Mode (Debug Logs)
To see detailed internal logs (tool calls, regime calculations, LLM prompts, etc.) during development:
```bash
RUST_LOG=trading_agent=debug cargo run
```

You can combine both: `RUST_LOG=trading_agent=debug cargo run --release`

## Following the Trading Journal

All decisions, executions, and errors are logged as structured JSON to `logs/trading-journal.log.YYYY-MM-DD` (daily rotation).

### View Real-time Logs

```bash
# Pretty console output (default)
cargo run --release

# Follow JSON logs
tail -f logs/trading-journal.log.* | jq .
```

### Useful jq queries

```bash
# All trading decisions
cat logs/trading-journal.log.* | jq 'select(.fields.event == "regime_interpretation")'

# Successful trade executions
cat logs/trading-journal.log.* | jq 'select(.fields.event == "trade_open_complete")'

# Errors only
cat logs/trading-journal.log.* | jq 'select(.level == "ERROR")'

# Position reversals
cat logs/trading-journal.log.* | jq 'select(.fields.event == "position_reversal")'
```

## Key Concepts Demonstrated

- **Agent Loop**: Manual implementation of the think-act-observe cycle
- **Tool Use**: Dynamic tool registration with JSON schema validation
- **Structured Output**: Structured LLM outputs and decision making
- **Error Handling**: Graceful degradation with exponential backoff
- **Secret Management**: Using `secrecy` crate for API keys
- **Structured Logging**: `tracing` for production-grade observability
- **Risk Management**: Built-in stop-loss and take-profit order placement
- **Async Rust**: Tokio-based concurrent API calls

## Disclaimer

This is an **educational project**. The trading strategies are for demonstration purposes only. **Always use the Kraken Demo environment for testing**. Never risk real capital on automated systems you don't fully understand.
