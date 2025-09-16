//! # Sniffer - Target Acquisition Module
//!
//! This module is the "ears" of the sniper bot. Its sole responsibility is to
//! connect to a Solana RPC node via WebSocket, subscribe to logs for the `pump.fun`
//! program, parse these logs to identify the creation of new tokens, and forward
//! them as `SnifferCandidate` objects to the main engine for execution.
//!
//! ## Architecture
//!
//! - **Resilient Connection:** It operates in a persistent loop, automatically handling
//!   disconnects and attempting to reconnect with an exponential backoff strategy. This
//!   ensures high availability even with unstable RPC nodes.
//!
//! - **Focused Parsing:** The log parsing logic is highly specific to the `pump.fun`
//!   program, using regular expressions to extract mint and creator public keys
//!   efficiently from `Create` instruction logs.
//!
//! - **Lightweight Deduplication:** A simple, time-bounded `HashSet` is used to
//!   prevent duplicate candidates from being sent downstream in quick succession,
//!   reducing the load on the engine.

use anyhow::Result;
use futures_util::stream::StreamExt;
use solana_client::{nonblocking::pubsub_client::PubsubClient, rpc_config::{RpcTransactionLogsConfig, RpcTransactionLogsFilter}};
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};
use std::{
    collections::HashSet,
    str::FromStr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, RwLock},
    task::JoinHandle,
    time,
};
use tracing::{debug, error, info, warn};

use crate::{config::Config, state::SnifferCandidate};

// The on-chain address for the pump.fun program.
const PUMP_FUN_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
const DEDUPLICATION_TTL: Duration = Duration::from_secs(60);

// A simple in-memory cache to avoid processing the same mint multiple times.
lazy_static::lazy_static! {
    static ref SEEN_MINTS: Mutex<(HashSet<String>, Instant)> = Mutex::new((HashSet::new(), Instant::now()));
}

/// Spawns the main sniffer task in the background.
pub fn spawn_sniffer_task(
    config: Arc<RwLock<Config>>,
    candidate_tx: mpsc::Sender<SnifferCandidate>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_wss_listener(config, candidate_tx).await;
    })
}

/// The main loop for the WebSocket listener.
async fn run_wss_listener(
    config: Arc<RwLock<Config>>,
    candidate_tx: mpsc::Sender<SnifferCandidate>,
) {
    info!("👃 Sniffer task started.");
    let mut reconnect_delay = Duration::from_millis(500);
    let max_reconnect_delay = Duration::from_secs(15);

    loop {
        let wss_url = {
            let cfg = config.read().await;
            cfg.wss_rpc_url.clone()
        };

        info!("Connecting to WSS endpoint: {}...", wss_url);
        match PubsubClient::new(&wss_url).await {
            Ok(client) => {
                info!("✅ Successfully connected to WSS endpoint.");
                reconnect_delay = Duration::from_millis(500); // Reset delay on successful connection

                let (mut stream, unsub) = client
                    .logs_subscribe(
                        RpcTransactionLogsFilter::Mentions(vec![PUMP_FUN_PROGRAM_ID.to_string()]),
                        RpcTransactionLogsConfig { commitment: Some(CommitmentConfig::processed()) },
                    )
                    .await
                    .expect("Failed to subscribe to logs");

                while let Some(log_info) = stream.next().await {
                    if let Some(candidate) = parse_pump_fun_logs(&log_info.value.logs) {
                        // Deduplicate candidates
                        let mut seen = SEEN_MINTS.lock().unwrap();
                        if seen.1.elapsed() > DEDUPLICATION_TTL {
                            seen.0.clear();
                            seen.1 = Instant::now();
                        }
                        if seen.0.insert(candidate.mint.to_string()) {
                            if let Err(e) = candidate_tx.send(candidate).await {
                                error!("Failed to send candidate to engine, channel closed: {}", e);
                                unsub().await;
                                return;
                            }
                        }
                    }
                }
                warn!("WSS stream ended. Reconnecting...");
                unsub().await;
            }
            Err(e) => {
                error!("Failed to connect to WSS endpoint: {}. Retrying in {:?}...", e, reconnect_delay);
                time::sleep(reconnect_delay).await;
                reconnect_delay = (reconnect_delay * 2).min(max_reconnect_delay);
            }
        }
    }
}

/// Parses logs from a transaction to find pump.fun token creation events.
fn parse_pump_fun_logs(logs: &[String]) -> Option<SnifferCandidate> {
    // A simple heuristic: the 'Create' log is a strong indicator.
    let creates_token = logs.iter().any(|log| log.contains("Instruction: Create"));
    if !creates_token {
        return None;
    }

    // Extract all pubkeys mentioned in the logs.
    let re = regex::Regex::new(r"[1-9A-HJ-NP-Za-km-z]{32,44}").unwrap();
    let keys: Vec<_> = logs
        .iter()
        .flat_map(|log| re.find_iter(log))
        .filter_map(|m| solana_sdk::pubkey::Pubkey::from_str(m.as_str()).ok())
        .collect();

    // In a pump.fun 'Create' transaction, the key layout is typically:
    // [0] - New Mint
    // [1] - Bonding Curve
    // [2] - Associated Bonding Curve
    // [3] - Creator
    if keys.len() >= 4 {
        let mint = keys[0];
        let creator = keys[3];
        debug!("Found new pump.fun mint: {}, Creator: {}", mint, creator);
        Some(SnifferCandidate {
            mint,
            creator,
            program_id: PUMP_FUN_PROGRAM_ID.to_string(),
        })
    } else {
        None
    }
}