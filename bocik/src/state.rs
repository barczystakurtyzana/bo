//! # Application State Management
//!
//! This module defines the core application state structures used throughout
//! the sniper bot to track the current mode and held tokens.

use serde::{Deserialize, Serialize};

/// The current operational mode of the bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// Listening for new token creation events.
    Sniffing,
    /// Currently holding a token and monitoring for sell opportunities.
    Holding,
}

/// Represents a token currently held by the bot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeldToken {
    /// The mint address of the token.
    pub mint: String,
    /// The amount of the token held.
    pub amount: f64,
}

/// The main application state structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppState {
    /// Current operational mode.
    pub mode: Mode,
    /// Currently held token, if any.
    pub held_token: Option<HeldToken>,
}

/// A candidate token detected by the sniffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnifferCandidate {
    /// The mint address of the new token.
    pub mint: String,
    /// The creator address of the token.
    pub creator: String,
    /// The program ID that created the token.
    pub program_id: String,
}