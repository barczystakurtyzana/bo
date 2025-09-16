//! # Solana Sniper Bot - Main Entrypoint
//!
//! This file is the orchestrator of the entire application. It is responsible for:
//! 1. Initializing all modules based on the configuration.
//! 2. Creating communication channels (`tokio::mpsc`) to wire them together.
//! 3. Spawning the primary components (Sniffer, Engine, UI Event Loop) as concurrent tasks.
//! 4. Launching the blocking GUI main loop.
//! 5. Ensuring a graceful shutdown of all tasks when the application exits.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::info;

// Declare the application modules based on our architecture plan.
mod config;
mod core;
mod engine;
mod sniffer;
mod state;
mod tx_builder;
mod ui;
mod wallet;

use crate::{
    config::Config,
    core::{nonce_manager::NonceManager, rpc_manager::RpcManager},
    engine::Engine,
    state::{AppState, Mode},
    ui::GuiEvent,
};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    // 1. Initialize logging infrastructure.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    info!("🚀 Initializing Solana Sniper Bot...");

    // 2. Load configuration and initialize shared state.
    let config = Arc::new(RwLock::new(Config::load()?));
    let app_state = Arc::new(RwLock::new(AppState {
        mode: Mode::Sniffing,
        held_token: None,
    }));

    // 3. Create communication channels.
    // Channel for sniffer to send potential buy candidates to the engine.
    let (candidate_tx, candidate_rx) = mpsc::channel(1024);
    // Channel for the UI to send commands (like 'sell') to the engine.
    let (ui_event_tx, ui_event_rx) = mpsc::channel(64);

    // 4. Initialize core execution components.
    info!("Initializing core execution engine...");
    let wallet_manager = Arc::new(wallet::WalletManager::from_config(&config.read().await)?);
    let rpc_manager = Arc::new(RpcManager::new(config.clone()));
    let nonce_manager = Arc::new(NonceManager::new(config.read().await.nonce_count)?);
    
    // The transaction builder needs the wallet to determine the signer.
    let transaction_builder = Arc::new(tx_builder::TransactionBuilder::new(wallet_manager.clone()));
    
    // Start the background monitoring task for the RPC manager.
    rpc_manager.start_monitoring().await;
    // Start the background reloading task for the nonce manager.
    // It requires an RPC client, so we get one from our manager.
    let rpc_client_for_nonce = rpc_manager.get_ranked_rpc_endpoints(1).await?.remove(0);
    nonce_manager.start_auto_reload(rpc_client_for_nonce).await;


    // 5. Spawn main application tasks.
    info!("Spawning main application tasks...");

    // Task for the sniffer.
    let sniffer_handle = sniffer::spawn_sniffer_task(config.clone(), candidate_tx);

    // Task for the main engine.
    let engine = Engine::new(
        config.clone(),
        app_state.clone(),
        rpc_manager.clone(),
        nonce_manager.clone(),
        transaction_builder.clone(),
        candidate_rx,
        ui_event_rx,
    );
    let engine_handle = engine.spawn_engine_task();

    // 6. Launch the GUI (this is a blocking call).
    info!("Launching User Interface...");
    ui::launch_gui(
        "Solana Sniper Bot",
        app_state.clone(),
        ui_event_tx,
    )?;

    // 7. Graceful shutdown on GUI exit.
    info!("GUI closed. Shutting down background tasks...");
    sniffer_handle.abort();
    engine_handle.abort();
    rpc_manager.stop_monitoring().await;

    info!("✅ Shutdown complete.");
    Ok(())
}
