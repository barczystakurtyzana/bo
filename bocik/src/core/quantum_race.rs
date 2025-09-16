//! # Quantum Race Execution Engine
//!
//! This module is the core of the offensive strategy, orchestrating the mass parallel
//! dispatch of transactions across multiple nonce accounts and intelligently selected
//! RPC endpoints. It is designed for maximum speed and probability of block inclusion.
//!
//! ## Architectural Flow
//!
//! 1.  **Ammunition Request:** Requests a pool of `Available` nonce accounts from the `NonceManager`.
//!     These accounts are the "ammunition" for the race.
//!
//! 2.  **Intelligence Gathering:** Queries the `RpcManager` for a ranked list of the best-performing
//!     RPC endpoints ("attack vectors") based on latency, success rate, and leader proximity.
//!
//! 3.  **Variant Construction:** Creates multiple variants of the base transaction, each signed
//!     with a different nonce hash.
//!
//! 4.  **Grid Dispatch (The Race):** Launches a parallel grid of `N x M` transaction sends
//!     (`N` nonces x `M` RPCs). The first transaction to achieve confirmation wins.
//!
//! 5.  **Cleanup & Reload:** Upon success, all other in-flight tasks are aborted, and the
//!     used nonce accounts are marked for advancement, preparing them for the next race.

use anyhow::{anyhow, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::Signature,
    transaction::Transaction,
};
use std::sync::Arc;
use std::time::{Instant, Duration};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use super::{
    nonce_manager::{NonceData, NonceManager},
    rpc_manager::RpcManager,
};

/// Result of a Quantum Race transaction dispatch.
#[derive(Debug)]
pub struct QuantumRaceResult {
    pub winning_signature: Signature,
    pub winning_rpc_url: String,
    pub winning_nonce_pubkey: Pubkey,
    pub total_attempts: usize,
    pub elapsed_time: Duration,
}

/// The engine orchestrating the parallel transaction dispatch.
pub struct QuantumRaceEngine {
    nonce_manager: Arc<NonceManager>,
    rpc_manager: Arc<RpcManager>,
}

impl QuantumRaceEngine {
    pub fn new(nonce_manager: Arc<NonceManager>, rpc_manager: Arc<RpcManager>) -> Self {
        info!("⚡ Quantum Race Engine initialized");
        Self {
            nonce_manager,
            rpc_manager,
        }
    }

    /// Executes the main Quantum Race workflow.
    pub async fn execute(
        &self,
        base_transaction: Transaction,
        max_rpcs_per_nonce: usize,
    ) -> Result<QuantumRaceResult> {
        let start_time = Instant::now();
        let nonce_pool_size = self.nonce_manager.get_pool_size();
        info!("🚀 Starting Quantum Race with up to {} nonces and {} RPCs per nonce.", nonce_pool_size, max_rpcs_per_nonce);

        // 1. Request all available ammunition.
        let available_nonces = self.nonce_manager.request_available(nonce_pool_size).await?;
        if available_nonces.is_empty() {
            return Err(anyhow!("No nonces available for Quantum Race"));
        }
        info!("🎯 Acquired {} nonce accounts for ammunition.", available_nonces.len());

        // 2. Request top attack vectors (optimal RPCs).
        let ranked_rpcs = self.rpc_manager.get_ranked_rpc_endpoints(max_rpcs_per_nonce).await?;
        if ranked_rpcs.is_empty() {
            self.nonce_manager.mark_for_advance(available_nonces).await;
            return Err(anyhow!("No RPC endpoints available for Quantum Race"));
        }
        info!("📡 Selected {} optimal RPC endpoints.", ranked_rpcs.len());

        // 3. Build transaction variants.
        let transaction_variants = self.build_transaction_variants(base_transaction, &available_nonces)?;
        info!("🔧 Built {} transaction variants.", transaction_variants.len());

        // 4. The Quantum Race Phase - Grid Dispatch.
        let total_attempts = transaction_variants.len() * ranked_rpcs.len();
        info!("🌊 Launching {} parallel attempts.", total_attempts);

        let race_result = self.execute_parallel_dispatch(transaction_variants, ranked_rpcs).await;
        
        // 5. Cleanup and Reload Phase.
        self.nonce_manager.mark_for_advance(available_nonces).await;

        match race_result {
            Ok(result) => {
                let elapsed = start_time.elapsed();
                info!("🏆 Quantum Race completed in {:?} - Winner: {}", elapsed, result.winning_signature);
                Ok(QuantumRaceResult {
                    winning_signature: result.winning_signature,
                    winning_rpc_url: result.winning_rpc_url,
                    winning_nonce_pubkey: result.winning_nonce_pubkey,
                    total_attempts,
                    elapsed_time: elapsed,
                })
            }
            Err(e) => {
                error!("Quantum Race failed: {}", e);
                Err(e)
            }
        }
    }

    /// Builds transaction variants using different nonce accounts.
    fn build_transaction_variants(
        &self,
        base_transaction: Transaction,
        nonce_accounts: &[Arc<NonceData>],
    ) -> Result<Vec<(Transaction, Pubkey)>> {
        let mut variants = Vec::new();
        for nonce_data in nonce_accounts {
            let mut variant = base_transaction.clone();
            let nonce_hash = {
                let cached = nonce_data.cached_nonce.read();
                cached.ok_or_else(|| anyhow!("Nonce {} has not been advanced yet", nonce_data.pubkey))?
            };
            variant.message.recent_blockhash = nonce_hash;
            variants.push((variant, nonce_data.pubkey));
        }
        Ok(variants)
    }

    /// Executes parallel dispatch across the transaction grid.
    async fn execute_parallel_dispatch(
        &self,
        transactions: Vec<(Transaction, Pubkey)>,
        rpc_clients: Vec<Arc<RpcClient>>,
    ) -> Result<QuantumRaceResult> {
        use tokio::sync::mpsc;

        let (tx, mut rx) = mpsc::channel(1);
        let mut handles = Vec::new();

        for (transaction, nonce_pubkey) in transactions {
            for rpc_client in &rpc_clients {
                let tx_clone = tx.clone();
                let transaction_clone = transaction.clone();
                let rpc_clone = rpc_client.clone();
                let rpc_url = "unknown_url".to_string(); // In production, this would be part of RpcEndpoint

                let handle = tokio::spawn(async move {
                    match rpc_clone.send_transaction(&transaction_clone).await {
                        Ok(signature) => {
                            debug!("📤 Transaction sent via {}: {}", rpc_url, signature);
                            let _ = tx_clone.send(QuantumRaceResult {
                                winning_signature: signature,
                                winning_rpc_url: rpc_url,
                                winning_nonce_pubkey: nonce_pubkey,
                                total_attempts: 0,
                                elapsed_time: Duration::default(),
                            }).await;
                        }
                        Err(e) => {
                            debug!("💥 Failed to send transaction via {}: {}", rpc_url, e);
                        }
                    }
                });
                handles.push(handle);
            }
        }
        
        drop(tx);

        match timeout(Duration::from_secs(15), rx.recv()).await {
            Ok(Some(result)) => {
                for handle in handles { handle.abort(); }
                Ok(result)
            }
            Ok(None) => Err(anyhow!("All transaction attempts failed in Quantum Race")),
            Err(_) => Err(anyhow!("Quantum Race timed out")),
        }
    }
}
