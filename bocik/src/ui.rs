//! # User Interface - Minimalist Control Panel
//!
//! This module provides a simple, lightweight graphical user interface (GUI) for the sniper bot
//! using the `eframe` and `egui` libraries. Its purpose is to serve as a command and control
//! panel for the operator, not a full-featured trading interface.
//!
//! ## Core Functionality
//!
//! 1.  **State Display:** Provides a real-time view of the bot's current state:
//!     - Current `Mode` (`Sniffing` or `Holding`).
//!     - The mint address and amount of the currently held token.
//!
//! 2.  **Manual Intervention:** Allows the operator to trigger a manual sell of the held token.
//!     - Provides buttons for selling predefined percentages (25%, 50%, 100%).
//!     - Supports keyboard shortcuts (W, Q, S) for rapid execution.
//!
//! 3.  **Asynchronous Communication:** The GUI runs on the main thread but communicates with the
//!     asynchronous `Engine` via an `mpsc` channel, ensuring that user actions do not block
//!     the core, time-sensitive operations of the bot.

use eframe::{
    egui::{self, CentralPanel, Key, Layout, RichText, Sense, Ui, Vec2},
    App, Frame,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, RwLock};
use tracing::info;

use crate::state::{AppState, Mode};

/// Events sent from the UI to the main Engine.
#[derive(Debug, Clone)]
pub enum GuiEvent {
    SellPercent(f64),
}

/// The main application struct for the egui frontend.
struct SniperApp {
    app_state: Arc<RwLock<AppState>>,
    ui_event_tx: mpsc::Sender<GuiEvent>,
}

impl SniperApp {
    fn new(app_state: Arc<RwLock<AppState>>, ui_event_tx: mpsc::Sender<GuiEvent>) -> Self {
        Self {
            app_state,
            ui_event_tx,
        }
    }

    /// Renders the main view based on the current application state.
    fn draw_main_view(&self, ui: &mut Ui, state: &AppState) {
        ui.heading(RichText::new("SNIPER BOT").size(24.0).strong());
        ui.separator();

        // --- Status Panel ---
        ui.group(|ui| {
            ui.label(RichText::new("STATUS").strong());
            match &state.mode {
                Mode::Sniffing => {
                    ui.horizontal(|ui| {
                        ui.label("Mode:");
                        ui.label(RichText::new("SNIFFING").color(egui::Color32::GREEN).strong());
                    });
                    ui.label("Awaiting new pump.fun tokens...");
                }
                Mode::Holding => {
                    ui.horizontal(|ui| {
                        ui.label("Mode:");
                        ui.label(RichText::new("HOLDING").color(egui::Color32::YELLOW).strong());
                    });
                    ui.add_space(10.0);
                    
                    if let Some(token) = &state.held_token {
                        ui.label("Mint:");
                        let mint_label = ui.add(egui::Label::new(token.mint.to_string()).sense(Sense::click()));
                        if mint_label.clicked() {
                            ui.output_mut(|o| o.copied_text = token.mint.to_string());
                        }
                        mint_label.on_hover_text("Click to copy");
                        ui.label(format!("Amount: {}", token.amount)); // Simple amount display
                    }
                }
            }
        });

        ui.add_space(15.0);

        // --- Action Panel (only visible when holding a token) ---
        if let Mode::Holding = state.mode {
            self.draw_sell_controls(ui);
        }
    }

    /// Renders the sell control buttons and shortcuts.
    fn draw_sell_controls(&self, ui: &mut Ui) {
        ui.group(|ui| {
            ui.label(RichText::new("ACTIONS").strong());
            ui.horizontal(|ui| {
                if ui.add_sized(Vec2::new(80.0, 30.0), egui::Button::new("Sell 25% (W)")).clicked() {
                    let _ = self.ui_event_tx.try_send(GuiEvent::SellPercent(0.25));
                }
                if ui.add_sized(Vec2::new(80.0, 30.0), egui::Button::new("Sell 50% (Q)")).clicked() {
                    let _ = self.ui_event_tx.try_send(GuiEvent::SellPercent(0.50));
                }
                if ui.add_sized(Vec2::new(80.0, 30.0), egui::Button::new("Sell 100% (S)")).clicked() {
                    let _ = self.ui_event_tx.try_send(GuiEvent::SellPercent(1.0));
                }
            });
            ui.label("Use keyboard shortcuts for faster execution.");
        });
    }
}

/// Implementation of the eframe::App trait for our UI.
impl App for SniperApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Handle keyboard shortcuts
        ctx.input(|i| {
            if i.key_pressed(Key::W) {
                let _ = self.ui_event_tx.try_send(GuiEvent::SellPercent(0.25));
            }
            if i.key_pressed(Key::Q) {
                let _ = self.ui_event_tx.try_send(GuiEvent::SellPercent(0.50));
            }
            if i.key_pressed(Key::S) {
                let _ = self.ui_event_tx.try_send(GuiEvent::SellPercent(1.0));
            }
        });

        CentralPanel::default().show(ctx, |ui| {
            // This is a blocking read, but it's acceptable for a UI that refreshes
            // periodically and is not on the critical path of the trading logic.
            let state = self.app_state.blocking_read().clone();
            self.draw_main_view(ui, &state);

            // Force a repaint to keep the UI responsive
            ctx.request_repaint_after(Duration::from_millis(100));
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        info!("👋 GUI closed by user.");
    }
}

/// Launches the GUI. This function will block the main thread.
pub fn launch_gui(
    app_state: Arc<RwLock<AppState>>,
    ui_event_tx: mpsc::Sender<GuiEvent>,
) -> anyhow::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([400.0, 250.0]),
        ..Default::default()
    };
    let app = SniperApp::new(app_state, ui_event_tx);

    eframe::run_native(
        "Sniper Bot Control Panel",
        native_options,
        Box::new(|_cc| Box::new(app)),
    )
    .map_err(|e| anyhow::anyhow!("GUI Error: {}", e))
}