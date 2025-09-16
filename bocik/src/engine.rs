//! # Main Bot Engine - The Brain of the Operation
//!
//! This module serves as the central nervous system for the sniper bot. It connects all other
//! components and implements the core logic loop, managing the bot's state (`Sniffing` vs. `Holding`)
//! and orchestrating the entire workflow from target acquisition to execution.
//!
//! ## Core Responsibilities
//!
//! 1.  **State Management:** Implements a simple but robust state machine.
//!     - In `Sniffing` mode, it actively listens for candidates from the sniffer.
//!     - In `Holding` mode, it ignores new candidates and waits for sell commands.
//!
//! 2.  **Workflow Orchestration:**
//!     - Consumes `SnifferCandidate` objects from the sniffer's channel.
//!     - Utilizes the `TransactionBuilder` to construct the raw buy/sell transactions.
//!     - Delegates the high-speed execution of these transactions to the `QuantumRaceEngine`.
//!     - Updates the shared `AppState` upon successful execution of buy or sell operations.
//!
//! 3.  **Command Handling:** Listens for `GuiEvent` commands (e.g., `SellPercent`) from the
//!     user interface and triggers the appropriate action.
//!
//! 4.  **Business Logic:** Enforces the "one token at a time" rule. After selling a sufficient
//!     percentage (>=98%) of the held token, it automatically transitions back to `Sniffing` mode.

use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::{
    config::Config,
    core::{
        nonce_manager::NonceManager, rpc_manager::RpcManager, quantum_race::QuantumRaceEngine,
    },
    state::{AppState, HeldToken, Mode, SnifferCandidate},
    tx_builder::TransactionBuilder,
    ui::GuiEvent,
};

/// The main engine that drives the bot's logic.
pub struct Engine {
    config: Arc<RwLock<Config>>,
    app_state: Arc<RwLock<AppState>>,
    quantum_race_engine: Arc<QuantumRaceEngine>,
    transaction_builder: Arc<TransactionBuilder>,
    candidate_rx: mpsc::Receiver<SnifferCandidate>,
    ui_event_rx: mpsc::Receiver<GuiEvent>,
}

impl Engine {
    pub fn new(
        config: Arc<RwLock<Config>>,
        app_state: Arc<RwLock<AppState>>,
        rpc_manager: Arc<RpcManager>,
        nonce_manager: Arc<NonceManager>,
        transaction_builder: Arc<TransactionBuilder>,
        candidate_rx: mpsc::Receiver<SnifferCandidate>,
        ui_event_rx: mpsc::Receiver<GuiEvent>,
    ) -> Self {
        let quantum_race_engine = Arc::new(QuantumRaceEngine::new(nonce_manager, rpc_manager));
        Self {
            config,
            app_state,
            quantum_race_engine,
            transaction_builder,
            candidate_rx,
            ui_event_rx,
        }
    }

    /// Spawns the main engine loop as a background task.
    pub fn spawn_engine_task(mut self) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!("⚙️ Engine task started.");
            self.run().await;
        })
    }

    /// The main logic loop of the bot.
    async fn run(&mut self) {
        loop {
            let mode = self.app_state.read().await.mode.clone();
            match mode {
                Mode::Sniffing => {
                    tokio::select! {
                        Some(candidate) = self.candidate_rx.recv() => {
                            self.handle_buy(candidate).await;
                        },
                        Some(event) = self.ui_event_rx.recv() => {
                            warn!("Received UI event {:?} while in Sniffing mode. Ignoring.", event);
                        }
                    }
                }
                Mode::Holding(_) => {
                     tokio::select! {
                        Some(candidate) = self.candidate_rx.recv() => {
                            // In a holding state, we ignore new candidates.
                            tracing::trace!("Ignoring candidate {:?} while holding a token.", candidate.mint);
                        },
                        Some(event) = self.ui_event_rx.recv() => {
                            if let GuiEvent::SellPercent(percent) = event {
                                self.handle_sell(percent).await;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Handles the autobuy process for a new candidate.
    async fn handle_buy(&self, candidate: SnifferCandidate) {
        info!("Handling BUY for candidate: {}", candidate.mint);
        let config = self.config.read().await;

        let base_tx = match self.transaction_builder.build_buy_transaction(&candidate, &config).await {
            Ok(tx) => tx,
            Err(e) => {
                error!("Failed to build buy transaction for {}: {}", candidate.mint, e);
                return;
            }
        };

        match self.quantum_race_engine.execute(base_tx, config.rpcs_per_nonce).await {
            Ok(result) => {
                info!("✅ Quantum Race BUY successful for {}: Signature {}", candidate.mint, result.winning_signature);
                let mut state = self.app_state.write().await;
                state.mode = Mode::Holding(candidate.mint);
                // In a real scenario, we'd get the actual amount of tokens bought from the transaction.
                // For now, we assume we bought a fixed amount.
                let amount_bought = config.buy_sol_amount * 1_000_000; // Simplified example
                state.held_token = Some(HeldToken {
                    mint: candidate.mint,
                    amount: amount_bought,
                });
            }
            Err(e) => {
                error!("❌ Quantum Race BUY failed for {}: {}", candidate.mint, e);
            }
        }
    }

    /// Handles the manual sell process initiated by the user.
    async fn handle_sell(&self, percent: f64) {
        let (mint, amount) = {
            let state = self.app_state.read().await;
            if let Some(token) = &state.held_token {
                (token.mint, (token.amount as f64 * percent).floor() as u64)
            } else {
                error!("Sell command received, but no token is held.");
                return;
            }
        };

        info!("Handling SELL for {}% of token {}", percent * 100.0, mint);
        let config = self.config.read().await;

        let base_tx = match self.transaction_builder.build_sell_transaction(&mint, amount, &config).await {
            Ok(tx) => tx,
            Err(e) => {
                error!("Failed to build sell transaction for {}: {}", mint, e);
                return;
            }
        };

        match self.quantum_race_engine.execute(base_tx, config.rpcs_per_nonce).await {
            Ok(result) => {
                info!("✅ Quantum Race SELL successful for {}: Signature {}", mint, result.winning_signature);
                let mut state = self.app_state.write().await;
                if let Some(token) = &mut state.held_token {
                    token.amount -= amount;
                    // If less than 2% of the token remains, consider it fully sold and return to sniffing.
                    if (token.amount as f64) / ((config.buy_sol_amount * 1_000_000) as f64) < 0.02 {
                        info!("Token {} fully sold. Returning to Sniffing mode.", mint);
                        state.mode = Mode::Sniffing;
                        state.held_token = None;
                    }
                }
            }
            Err(e) => {
                error!("❌ Quantum Race SELL failed for {}: {}", mint, e);
            }
        }
    }
}