//! # Central Configuration Module
//!
//! This module defines the global configuration structure for the sniper bot.
//! It uses `serde` to deserialize a `config.toml` file into a strongly-typed
//! `Config` struct.
//!
//! ## Core Principles
//!
//! - **Centralization:** All user-configurable parameters are defined in one place.
//! - **Resilience:** The bot can run with a minimal or even non-existent config file
//!   by falling back to sensible default values for every parameter.
//! - **Hot-Reload Ready:** The structure is designed to be wrapped in an `Arc<RwLock<>>`,
//!   allowing for runtime configuration changes if needed in the future.

use serde::Deserialize;
use std::fs;
use tracing::warn;

/// Defines the scoring weights for the RpcManager's adaptive ranking algorithm.
/// These values are loaded from the `[scoring_weights]` table in `config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct ScoringWeights {
    #[serde(default = "default_latency_weight")]
    pub latency_weight: f64,
    #[serde(default = "default_success_rate_weight")]
    pub success_rate_weight: f64,
    #[serde(default = "default_geo_weight")]
    pub geo_weight: f64,
    #[serde(default = "default_stake_weight")]
    pub stake_weight: f64,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            latency_weight: default_latency_weight(),
            success_rate_weight: default_success_rate_weight(),
            geo_weight: default_geo_weight(),
            stake_weight: default_stake_weight(),
        }
    }
}

/// The main configuration structure for the application.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    // -- Core Settings --
    #[serde(default)]
    pub keypair_path: String,

    // -- Network Settings --
    #[serde(default = "default_rpc_url")]
    pub rpc_url: String,
    #[serde(default = "default_wss_rpc_url")]
    pub wss_rpc_url: String,
    #[serde(default = "default_rpc_endpoints")]
    pub rpc_endpoints: Vec<String>,

    // -- Quantum Race Engine Settings --
    #[serde(default = "default_nonce_count")]
    pub nonce_count: usize,
    #[serde(default = "default_rpcs_per_nonce")]
    pub rpcs_per_nonce: usize,

    // -- Trading Parameters --
    #[serde(default = "default_buy_sol_amount")]
    pub buy_sol_amount: f64,
    #[serde(default = "default_priority_fee_microlamports")]
    pub priority_fee_microlamports: u64,

    // -- RpcManager Scoring Weights --
    #[serde(default)]
    pub scoring_weights: ScoringWeights,
}

impl Config {
    /// Loads configuration from `config.toml`.
    /// If the file doesn't exist or fails to parse, it returns a default configuration.
    pub fn load() -> Self {
        match fs::read_to_string("config.toml") {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => config,
                Err(e) => {
                    warn!("Failed to parse config.toml: {}. Using default values.", e);
                    Config::default()
                }
            },
            Err(_) => {
                warn!("config.toml not found. Using default values.");
                Config::default()
            }
        }
    }
}

// --- Default value functions for serde ---

fn default_rpc_url() -> String { "https://solana-devnet.g.alchemy.com/v2/rWsbieEnAlFuYs-KKUfv1".to_string() }
fn default_wss_rpc_url() -> String { "wss://devnet.helius-rpc.com/?api-key=18435404-2f2b-4046-b273-f66b7c488b9c".to_string() }
fn default_rpc_endpoints() -> Vec<String> { vec![default_rpc_url()] }
fn default_nonce_count() -> usize { 5 }
fn default_rpcs_per_nonce() -> usize { 3 }
fn default_buy_sol_amount() -> f64 { 0.05 }
fn default_priority_fee_microlamports() -> u64 { 500_000 }

// Scoring weights defaults
fn default_latency_weight() -> f64 { 2.0 }
fn default_success_rate_weight() -> f64 { 2.5 }
fn default_geo_weight() -> f64 { 1.5 }
fn default_stake_weight() -> f64 { 1.0 }