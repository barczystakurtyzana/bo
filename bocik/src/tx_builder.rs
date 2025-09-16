//! # Transaction Builder
//!
//! This module is responsible for constructing the specific instructions and
//! transactions required to interact with the pump.fun platform. It uses the
//! `pumpfun` SDK crate to ensure correctness and compatibility.

use anyhow::Result;
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    message::{v0::Message as MessageV0, VersionedMessage},
    pubkey::Pubkey,
    transaction::{Transaction as LegacyTransaction, VersionedTransaction},
};
use std::sync::Arc;

use crate::{config::Config, state::SnifferCandidate, wallet::WalletManager};

// Import the pumpfun SDK, enabled via feature flag in Cargo.toml
#[cfg(feature = "pumpfun")]
use pumpfun::{PumpFun, common::types::PriorityFee};

pub struct TransactionBuilder {
    wallet: Arc<WalletManager>,
    // In a future version, a shared pumpfun client could be initialized here.
}

impl TransactionBuilder {
    pub fn new(wallet: Arc<WalletManager>) -> Self {
        Self { wallet }
    }

    /// Builds a complete, unsigned transaction to buy a token on pump.fun.
    /// The transaction will be fully signed later by the QuantumRaceEngine using the
    /// appropriate nonce authority and the user's wallet.
    #[cfg(feature = "pumpfun")]
    pub async fn build_buy_transaction(
        &self,
        candidate: &SnifferCandidate,
        config: &Config,
    ) -> Result<LegacyTransaction> {
        let pumpfun_client = PumpFun::new(self.wallet.keypair.clone()).await?;

        // Calculate the minimum token output based on slippage tolerance.
        // NOTE: This requires an RPC call to get the bonding curve state.
        let bonding_curve = pumpfun_client.get_bonding_curve(candidate.mint).await?;
        let virtual_sol_reserves = bonding_curve.virtual_sol_reserves;
        let virtual_token_reserves = bonding_curve.virtual_token_reserves;

        let expected_tokens = (config.buy_sol_amount * virtual_token_reserves) / (virtual_sol_reserves + config.buy_sol_amount);
        let min_token_out = (expected_tokens as u128 * (10000 - config.slippage_bps as u128) / 10000) as u64;

        // Construct the transaction using the SDK.
        let tx = pumpfun_client
            .buy(
                candidate.mint,
                config.buy_sol_amount,
                Some(min_token_out),
                Some(PriorityFee::from(config.priority_fee_lamports)),
            )
            .await?;

        // We return a legacy transaction here because the QuantumRaceEngine will re-sign it with a nonce.
        // The SDK returns a VersionedTransaction, so we need to convert it.
        let legacy_tx = match tx.message {
            VersionedMessage::V0(msg) => {
                LegacyTransaction::new_with_payer(&msg.instructions, Some(&self.wallet.pubkey()))
            }
            VersionedMessage::Legacy(msg) => {
                 LegacyTransaction::new_with_payer(&msg.instructions, Some(&self.wallet.pubkey()))
            }
        };

        Ok(legacy_tx)
    }
    
    /// Builds a sell transaction for pump.fun.
    #[cfg(feature = "pumpfun")]
    pub async fn build_sell_transaction(
        &self,
        mint: &Pubkey,
        token_amount: u64,
        config: &Config,
    ) -> Result<LegacyTransaction> {
        let pumpfun_client = PumpFun::new(self.wallet.keypair.clone()).await?;

        let bonding_curve = pumpfun_client.get_bonding_curve(*mint).await?;
        let virtual_sol_reserves = bonding_curve.virtual_sol_reserves;
        let virtual_token_reserves = bonding_curve.virtual_token_reserves;

        let expected_sol = (token_amount * virtual_sol_reserves) / (virtual_token_reserves + token_amount);
        let min_sol_output = (expected_sol as u128 * (10000 - config.slippage_bps as u128) / 10000) as u64;
        
        let tx = pumpfun_client
            .sell(
                *mint,
                Some(token_amount),
                Some(min_sol_output),
                Some(PriorityFee::from(config.priority_fee_lamports))
            )
            .await?;

        let legacy_tx = match tx.message {
            VersionedMessage::V0(msg) => {
                LegacyTransaction::new_with_payer(&msg.instructions, Some(&self.wallet.pubkey()))
            }
            VersionedMessage::Legacy(msg) => {
                 LegacyTransaction::new_with_payer(&msg.instructions, Some(&self.wallet.pubkey()))
            }
        };

        Ok(legacy_tx)
    }


    // Fallback implementation for when the 'pumpfun' feature is not enabled.
    // This allows the bot to compile and run in a limited capacity.
    #[cfg(not(feature = "pumpfun"))]
    pub async fn build_buy_transaction(
        &self,
        _candidate: &SnifferCandidate,
        _config: &Config,
    ) -> Result<LegacyTransaction> {
        tracing::warn!("'pumpfun' feature is not enabled. Using placeholder transaction.");
        // Create a simple memo instruction as a placeholder.
        let memo_ix = spl_memo::build_memo(b"pumpfun buy placeholder", &[&self.wallet.pubkey()]);
        Ok(LegacyTransaction::new_with_payer(&[memo_ix], Some(&self.wallet.pubkey())))
    }
    
    #[cfg(not(feature = "pumpfun"))]
    pub async fn build_sell_transaction(
        &self,
        _mint: &Pubkey,
        _token_amount: u64,
        _config: &Config,
    ) -> Result<LegacyTransaction> {
        tracing::warn!("'pumpfun' feature is not enabled. Using placeholder transaction.");
        let memo_ix = spl_memo::build_memo(b"pumpfun sell placeholder", &[&self.wallet.pubkey()]);
        Ok(LegacyTransaction::new_with_payer(&[memo_ix], Some(&self.wallet.pubkey())))
    }
}
