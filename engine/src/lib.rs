//! Deterministic OpenDominion protection-period engine (pinned to round-50).
//!
//! A faithful, RNG-free port of the game's per-tick logic and protection actions,
//! validated bit-for-bit against golden vectors emitted by the PHP oracle
//! (see engine/tests/golden/).

// ───────── the protection-period SIMULATE + game-mechanic engine (bit-exact vs round-50) ─────────
pub mod calc;
pub mod combat;
pub mod config;
pub mod data;
pub mod networth;
pub mod plan;
pub mod race_resources;
pub mod rounding;
pub mod state;
pub mod tick;
