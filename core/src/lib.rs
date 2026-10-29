//! Authenticity: an Obsidian plugin in Rust. Its checks run on the user's own computer (GPTZero, optional, online, only
//! when the user links a key and chooses it for a check: gptzero):
//!
//! - **AI writing** (ai, sloptotal): a note's or a selection's paragraphs scored by SlopTotal, a local detector that
//!   Authenticity's helper (../native) starts on demand and stops when idle; the detectors' reasons and a sentence's
//!   own score are asked for from the AI Detector pane (ai_table), with model-free hints (hints);
//! - **plagiarism** (similarity, library, index): wording shared with the library notes, from a shingle index kept up
//!   to date note by note (../similar), and close paraphrases (para) by the index's content-word search;
//! - **AI characters** (chars): invisible and look-alike characters, by fixed rules.
//!
//! main.js (the only JavaScript, a few lines) loads this module and calls `start` (app). The results are drawn in the
//! editor and Reading view through the workspace's shared view-marks crate (render), listed in two side panes, AI Detector and Plagiarism (panel; the comparison table is table),
//! and offered to other plugins (api). Nothing is ever rewritten: the checks mark and explain.

pub use obsidian_kit::js; // shared with the other plugins (../../shared)

pub mod ai;
pub mod ai_table;
pub mod align;
mod api;
mod app;
pub mod chars;
pub mod clean;
mod helper;
pub mod gptzero;
pub mod hints;
mod journal;
mod logview;
mod index;
pub mod library;
mod panel;
pub use authenticity_similar::para;
pub mod paths;
pub mod prose;
mod render;
pub mod similarity;
pub mod sloptotal;
pub mod table;
pub mod walk;
