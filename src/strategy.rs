//! Strategy interfaces.
//!
//! Every strategy is *incremental*: it is fed one day at a time, after the close,
//! and states the portfolio it wants to hold. The engine executes that at the
//! **next** day's open. Strategies never see future bars, so lookahead bias is
//! impossible by construction.

use crate::data::Bar;
use chrono::NaiveDate;
use std::sync::Arc;

/// Portfolio-level strategy: sees every symbol each day (cross-sectional methods).
pub trait Strategy: Send {
    /// `bars[i]` is today's bar for symbol `i` (None if it did not trade).
    /// `weights` arrives filled with NaN, meaning "no change". Set an entry to a
    /// number to request a new target weight for that symbol. The engine clamps
    /// each weight to [-1, 1] and scales so sum(|w|) <= 1.
    fn next(&mut self, date: NaiveDate, bars: &[Option<Bar>], weights: &mut [f64]);
}

/// Single-symbol strategy: returns the desired exposure in [-1, 1] for one asset.
pub trait SymbolStrategy: Send {
    fn next(&mut self, date: NaiveDate, bar: &Bar) -> f64;
}

/// Runs an independent `SymbolStrategy` per symbol, equal capital per listed symbol.
pub struct PerSymbol {
    make: Arc<dyn Fn() -> Box<dyn SymbolStrategy> + Send + Sync>,
    inst: Vec<Option<Box<dyn SymbolStrategy>>>,
    exposure: Vec<f64>,
}

impl Strategy for PerSymbol {
    fn next(&mut self, date: NaiveDate, bars: &[Option<Bar>], weights: &mut [f64]) {
        if self.inst.len() != bars.len() {
            self.inst.resize_with(bars.len(), || None);
            self.exposure.resize(bars.len(), 0.0);
        }
        let mut changed = vec![false; bars.len()];
        for (i, b) in bars.iter().enumerate() {
            if let Some(b) = b {
                let s = self.inst[i].get_or_insert_with(|| (self.make)());
                let e = s.next(date, b);
                let e = if e.is_finite() {
                    e.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                changed[i] = e != self.exposure[i];
                self.exposure[i] = e;
            }
        }
        // Only re-target symbols whose exposure changed, so a constant signal
        // (e.g. buy and hold) lets the position ride instead of rebalancing daily.
        let listed = self.inst.iter().filter(|s| s.is_some()).count().max(1) as f64;
        for (i, w) in weights.iter_mut().enumerate() {
            if changed[i] {
                *w = self.exposure[i] / listed;
            }
        }
    }
}

pub type Builder = Arc<dyn Fn() -> Box<dyn Strategy> + Send + Sync>;

/// A named, parameterised strategy that can be instantiated any number of times.
#[derive(Clone)]
pub struct Spec {
    /// Stable unique id, e.g. `trend.ma_cross.ema.10x50.lo`. Never rename: the paper ledger keys on it.
    pub id: String,
    /// Broad method family, e.g. `trend`, `mean_reversion`, `learning`.
    pub family: &'static str,
    pub description: String,
    pub build: Builder,
}

impl Spec {
    pub fn portfolio<F, S>(
        id: impl Into<String>,
        family: &'static str,
        desc: impl Into<String>,
        f: F,
    ) -> Spec
    where
        F: Fn() -> S + Send + Sync + 'static,
        S: Strategy + 'static,
    {
        Spec {
            id: id.into(),
            family,
            description: desc.into(),
            build: Arc::new(move || Box::new(f())),
        }
    }

    pub fn per_symbol<F, S>(
        id: impl Into<String>,
        family: &'static str,
        desc: impl Into<String>,
        f: F,
    ) -> Spec
    where
        F: Fn() -> S + Send + Sync + 'static,
        S: SymbolStrategy + 'static,
    {
        let make: Arc<dyn Fn() -> Box<dyn SymbolStrategy> + Send + Sync> =
            Arc::new(move || Box::new(f()));
        Spec {
            id: id.into(),
            family,
            description: desc.into(),
            build: Arc::new(move || {
                Box::new(PerSymbol {
                    make: make.clone(),
                    inst: Vec::new(),
                    exposure: Vec::new(),
                })
            }),
        }
    }
}

/// Long-only or long/short variant of a directional signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    LongOnly,
    LongShort,
}
impl Side {
    pub const BOTH: [Side; 2] = [Side::LongOnly, Side::LongShort];
    pub fn tag(self) -> &'static str {
        match self {
            Side::LongOnly => "lo",
            Side::LongShort => "ls",
        }
    }
    pub fn words(self) -> &'static str {
        match self {
            Side::LongOnly => "long-only",
            Side::LongShort => "long/short",
        }
    }
    /// Map a raw directional signal to an allowed exposure.
    pub fn apply(self, x: f64) -> f64 {
        match self {
            Side::LongOnly => x.max(0.0),
            Side::LongShort => x,
        }
    }
}
