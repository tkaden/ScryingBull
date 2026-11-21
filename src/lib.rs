//! Scrying Bull: run hundreds of very different trading methods side by side.
//!
//! Pipeline: [`data`] -> [`strategies`] (via the [`strategy`] traits) ->
//! [`engine`] (fills and costs) -> [`backtest`] / [`paper`] -> [`site`].

// Numeric code indexes several parallel arrays at once; ranges read better there.
#![allow(clippy::needless_range_loop)]

pub mod analyze;
pub mod backtest;
pub mod broker;
pub mod config;
pub mod data;
pub mod engine;
pub mod indicators;
pub mod live;
pub mod metrics;
pub mod paper;
pub mod rng;
pub mod site;
pub mod strategies;
pub mod strategy;
