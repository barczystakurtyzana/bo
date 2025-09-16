//! # Core Execution Engine
//!
//! This module consolidates the high-performance components responsible for
//! the reliable and rapid execution of transactions. It represents the "engine room"
//! of the sniper bot.
//!
//! ## Sub-modules
//!
//! - **`nonce_manager`**: The "Ammunition Magazine". Manages a pool of durable nonce
//!   accounts to provide independent, ready-to-use transaction streams.
//!
//! - **`rpc_manager`**: The "Intelligence Center". Monitors, ranks, and selects the
//!   most optimal RPC endpoints for transaction submission based on a strategic
//!   scoring algorithm.
//!
//! - **`quantum_race`**: The "Execution Engine". Orchestrates the mass parallel
//!   dispatch of transactions across the best nonces and RPCs to maximize the
//!   speed and probability of success.

pub mod durable_nonce_manager;
pub mod nonce_manager {
    pub use super::durable_nonce_manager::*;
}
pub mod rpc_manager;
pub mod quantum_race;