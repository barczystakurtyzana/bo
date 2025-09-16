//! # Nonce Manager - The Ammunition Magazine
//!
//! Manages a pool of durable nonce accounts, which act as independent, ready-to-use
//! transaction streams ("ammunition") for the Quantum Race Engine. This architecture
//! is critical for high-throughput, parallel transaction submission without conflicts.
//!
//! ## Core Concepts
//!
//! - **State Machine:** Each nonce account is managed by a strict state machine
//!   (`Available` -> `InFlight` -> `RequiresAdvance`) to ensure it's only used when valid.
//!
//! - **Background Reloading:** A dedicated background task handles the `advance_nonce`
//!   instruction, preparing used nonces for their next use without blocking the critical path.
//!
//! - **Performance:** Uses high-performance `parking_lot` locks for state transitions,
//!   decoupling the fast acquisition path from the slower, asynchronous network operations.

use anyhow::{anyhow, Result};
use parking_lot::{Mutex, RwLock};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    hash::Hash, instruction::Instruction, message::Message, pubkey::Pubkey, signature::Keypair,
    signer::Signer, system_instruction, transaction::Transaction,
};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

/// State of a nonce account in the Quantum Race Architecture.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NonceState {
    /// Ready for immediate use with a fresh nonce hash.
    Available,
    /// Used in a transaction batch, locked until confirmation or forced advance.
    InFlight,
    /// Transaction confirmed, queued for `advance_nonce` to become `Available` again.
    RequiresAdvance,
}

#[derive(Debug)]
pub struct NonceData {
    pub pubkey: Pubkey,
    pub authority: Arc<Keypair>,
    pub cached_nonce: RwLock<Option<Hash>>,
    pub state: RwLock<NonceState>,
    pub last_used: RwLock<Option<Instant>>,
}

/// Manages a pool of ready-to-use, independent transaction streams.
#[derive(Debug)]
pub struct NonceManager {
    pool: Vec<Arc<NonceData>>,
    /// Atomic counter for Round-Robin selection of available accounts.
    next_available_index: AtomicUsize,
    /// Background task handle for auto-reloading (nonce advance).
    reload_task_handle: Mutex<Option<JoinHandle<()>>>,
    /// Queue for accounts that require advancement.
    advance_queue: Arc<Mutex<Vec<Arc<NonceData>>>>,
}

impl NonceManager {
    /// Creates a NonceManager with randomly generated, non-seeded nonce accounts.
    pub fn new(nonce_count: usize) -> Result<Self> {
        if nonce_count == 0 {
            return Err(anyhow!("Nonce pool cannot be empty"));
        }

        let pool = (0..nonce_count)
            .map(|_| {
                let authority = Arc::new(Keypair::new());
                let nonce_keypair = Keypair::new();

                Arc::new(NonceData {
                    pubkey: nonce_keypair.pubkey(),
                    authority,
                    cached_nonce: RwLock::new(None),
                    state: RwLock::new(NonceState::RequiresAdvance), // Start in advance state
                    last_used: RwLock::new(None),
                })
            })
            .collect();

        info!(
            "🎯 NonceManager initialized with {} non-seeded accounts",
            nonce_count
        );

        Ok(Self {
            pool,
            next_available_index: AtomicUsize::new(0),
            reload_task_handle: Mutex::new(None),
            advance_queue: Arc::new(Mutex::new(Vec::new())),
        })
    }
    
    /// Starts the background auto-reloading loop for nonce advancement.
    pub async fn start_auto_reload(&self, rpc_client: Arc<RpcClient>) {
        let advance_queue = self.advance_queue.clone();
        let pool = self.pool.clone();

        let handle = tokio::spawn(async move {
            info!("🔄 NonceManager auto-reload loop started");
            let mut retry_delay = Duration::from_millis(100);
            const MAX_RETRY_DELAY: Duration = Duration::from_secs(10);

            loop {
                let accounts_to_advance = {
                    let mut queue = advance_queue.lock();
                    queue.drain(..).collect::<Vec<_>>()
                };

                if !accounts_to_advance.is_empty() {
                    debug!("🔄 Processing {} accounts for advance", accounts_to_advance.len());
                }

                let mut failed_accounts = Vec::new();
                for nonce_data in accounts_to_advance {
                    match Self::advance_nonce(&rpc_client, &nonce_data).await {
                        Ok(()) => {
                            retry_delay = Duration::from_millis(100);
                        }
                        Err(e) => {
                            error!("Failed to advance nonce for {}: {}", nonce_data.pubkey, e);
                            failed_accounts.push(nonce_data);
                        }
                    }
                }

                if !failed_accounts.is_empty() {
                    {
                        let mut queue = advance_queue.lock();
                        queue.extend(failed_accounts);
                    }
                    let jitter = Duration::from_millis(fastrand::u64(0..50));
                    tokio::time::sleep(retry_delay + jitter).await;
                    retry_delay = std::cmp::min(retry_delay * 2, MAX_RETRY_DELAY);
                } else {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                
                // Check for stuck InFlight accounts
                for nonce_data in &pool {
                     let should_force_advance = {
                        let state = nonce_data.state.read();
                        if *state == NonceState::InFlight {
                            if let Some(last_used) = *nonce_data.last_used.read() {
                                last_used.elapsed() > Duration::from_secs(30)
                            } else { false }
                        } else { false }
                    };
                    if should_force_advance {
                        warn!("🚨 Force advancing stuck nonce: {}", nonce_data.pubkey);
                        let mut queue = advance_queue.lock();
                        queue.push(nonce_data.clone());
                    }
                }
            }
        });

        *self.reload_task_handle.lock() = Some(handle);
    }


    /// Requests multiple available nonces for the Quantum Race.
    pub async fn request_available(&self, count: usize) -> Result<Vec<Arc<NonceData>>> {
        let mut available_nonces = Vec::new();
        let mut attempts = 0;
        let max_attempts = self.pool.len() * 2;

        while available_nonces.len() < count && attempts < max_attempts {
            let index = self.next_available_index.fetch_add(1, Ordering::Relaxed) % self.pool.len();
            let nonce_data = &self.pool[index];

            {
                let mut state = nonce_data.state.write();
                if *state == NonceState::Available {
                    *state = NonceState::InFlight;
                    *nonce_data.last_used.write() = Some(Instant::now());
                    available_nonces.push(nonce_data.clone());
                    debug!("🎯 Acquired nonce {} for Quantum Race", nonce_data.pubkey);
                }
            }
            attempts += 1;
        }

        if available_nonces.len() < count {
            warn!("⚠️ Could only acquire {}/{} requested nonces", available_nonces.len(), count);
        }

        Ok(available_nonces)
    }

    /// Marks nonces as requiring advancement after transaction confirmation.
    pub async fn mark_for_advance(&self, nonce_accounts: Vec<Arc<NonceData>>) {
        let mut queue = self.advance_queue.lock();
        for nonce_data in nonce_accounts {
            let mut state = nonce_data.state.write();
            if *state == NonceState::InFlight {
                *state = NonceState::RequiresAdvance;
                queue.push(nonce_data.clone());
                debug!("⏳ Nonce {} queued for advance", nonce_data.pubkey);
            }
        }
    }

    /// Internal method to advance a nonce and update its state.
    async fn advance_nonce(rpc_client: &RpcClient, nonce_data: &NonceData) -> Result<()> {
        let advance_ix = system_instruction::advance_nonce_account(
            &nonce_data.pubkey,
            &nonce_data.authority.pubkey(),
        );
        let recent_blockhash = rpc_client.get_latest_blockhash().await?;
        let message = Message::new(&[advance_ix], Some(&nonce_data.authority.pubkey()));
        let mut transaction = Transaction::new_unsigned(message);
        transaction.sign(&[nonce_data.authority.as_ref()], recent_blockhash);

        let signature = rpc_client.send_and_confirm_transaction(&transaction).await?;

        let nonce_account = rpc_client.get_account(&nonce_data.pubkey).await?;
        
        use solana_sdk::nonce::state::{State, Versions};
        let nonce_hash = match bincode::deserialize::<Versions>(&nonce_account.data) {
            Ok(Versions::Current(state)) => match *state {
                State::Initialized(data) => data.blockhash(),
                _ => return Err(anyhow!("Nonce account is not initialized")),
            },
            _ => return Err(anyhow!("Failed to parse nonce account data")),
        };
        
        {
            *nonce_data.cached_nonce.write() = Some(nonce_hash);
            *nonce_data.state.write() = NonceState::Available;
        }
        debug!("✅ Nonce {} advanced successfully: {}", nonce_data.pubkey, signature);
        Ok(())
    }

    /// Returns the total number of nonces in the pool.
    pub fn get_pool_size(&self) -> usize {
        self.pool.len()
    }
}
