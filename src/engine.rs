//! Execution simulator shared by backtests and the paper ledger.
//!
//! Timeline per day `t`: (1) fill outstanding target changes at today's open,
//! with slippage and commission; (2) mark to market at the close; (3) the
//! strategy sees today's close and may change targets for tomorrow.

use crate::data::{Bar, Panel};
use crate::strategy::Strategy;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Costs {
    /// Commission per trade, basis points of notional.
    pub commission_bps: f64,
    /// Adverse slippage vs. the open, basis points.
    pub slippage_bps: f64,
    /// Ignore rebalances smaller than this fraction of equity (avoids churn).
    pub min_trade: f64,
}

impl Default for Costs {
    fn default() -> Self {
        Costs {
            commission_bps: 1.0,
            slippage_bps: 5.0,
            min_trade: 0.005,
        }
    }
}

/// Cash + positions + the strategy's standing targets.
#[derive(Clone, Debug)]
pub struct Book {
    pub cash: f64,
    pub qty: Vec<f64>,
    /// Last known price per symbol (for valuing positions on days without a bar).
    pub last_px: Vec<f64>,
    /// Most recent explicit target weight per symbol.
    pub target: Vec<f64>,
    /// Target changed and not yet executed.
    pub dirty: Vec<bool>,
    pub trades: u64,
    /// Cumulative traded notional / equity.
    pub turnover: f64,
}

impl Book {
    pub fn new(cash: f64, n: usize) -> Self {
        Book {
            cash,
            qty: vec![0.0; n],
            last_px: vec![0.0; n],
            target: vec![0.0; n],
            dirty: vec![false; n],
            trades: 0,
            turnover: 0.0,
        }
    }

    fn equity_with(&self, px: impl Fn(usize) -> f64) -> f64 {
        self.cash
            + self
                .qty
                .iter()
                .enumerate()
                .map(|(i, q)| if *q == 0.0 { 0.0 } else { q * px(i) })
                .sum::<f64>()
    }

    pub fn equity(&self) -> f64 {
        self.equity_with(|i| self.last_px[i])
    }

    /// Record new targets from a strategy (NaN = no change).
    pub fn set_targets(&mut self, w: &[f64]) {
        for (i, x) in w.iter().enumerate() {
            if x.is_finite() {
                self.target[i] = x.clamp(-1.0, 1.0);
                self.dirty[i] = true;
            }
        }
    }

    /// Mark every non-flat target for execution (used when trading starts).
    pub fn arm_all(&mut self) {
        for i in 0..self.target.len() {
            self.dirty[i] |= self.target[i] != 0.0 || self.qty[i] != 0.0;
        }
    }

    /// Execute outstanding target changes at today's open.
    pub fn execute(&mut self, bars: &[Option<Bar>], costs: &Costs) {
        if !self.dirty.iter().any(|d| *d) {
            return;
        }
        let open = |i: usize| bars[i].map_or(self.last_px[i], |b| b.open);
        let equity = self.equity_with(open);
        if equity <= 0.0 {
            return;
        }
        // Enforce gross exposure <= 1 using targets where dirty, current weights elsewhere.
        let gross: f64 = (0..self.qty.len())
            .map(|i| {
                if self.dirty[i] {
                    self.target[i].abs()
                } else {
                    (self.qty[i] * open(i) / equity).abs()
                }
            })
            .sum();
        if gross > 1.0 + 1e-9 {
            for i in 0..self.qty.len() {
                if !self.dirty[i] {
                    self.target[i] = self.qty[i] * open(i) / equity;
                }
                self.target[i] /= gross;
                self.dirty[i] = true;
            }
        }
        for (i, bar) in bars.iter().enumerate() {
            let Some(bar) = bar else { continue }; // can't trade without a market
            if !self.dirty[i] {
                continue;
            }
            self.dirty[i] = false;
            let px = bar.open;
            let delta = self.target[i] * equity - self.qty[i] * px;
            let closing = self.target[i] == 0.0 && self.qty[i] != 0.0;
            if !closing && delta.abs() < costs.min_trade * equity {
                continue;
            }
            let fill = px * (1.0 + delta.signum() * costs.slippage_bps * 1e-4);
            let shares = if closing { -self.qty[i] } else { delta / fill };
            let notional = shares * fill;
            self.qty[i] = if closing { 0.0 } else { self.qty[i] + shares };
            self.cash -= notional + notional.abs() * costs.commission_bps * 1e-4;
            self.trades += 1;
            self.turnover += notional.abs() / equity;
        }
    }

    /// Update prices with today's closes and return equity.
    pub fn mark(&mut self, bars: &[Option<Bar>]) -> f64 {
        for (i, b) in bars.iter().enumerate() {
            if let Some(b) = b {
                self.last_px[i] = b.close;
            }
        }
        self.equity()
    }

    /// Current weights at last known prices.
    pub fn weights(&self) -> Vec<f64> {
        let eq = self.equity();
        self.qty
            .iter()
            .zip(&self.last_px)
            .map(|(q, p)| if eq > 0.0 { q * p / eq } else { 0.0 })
            .collect()
    }
}

/// Result of simulating one strategy over a panel.
pub struct Run {
    pub equity: Vec<(NaiveDate, f64)>,
    pub book: Book,
    /// Average gross exposure over the evaluation window.
    pub avg_exposure: f64,
    /// Gross exposure (sum of |weights|) at each recorded close.
    pub exposure: Vec<f64>,
}

/// Simulate `strategy` over `panel`. The strategy is fed every bar (warm-up
/// included) but trading and equity recording start at `eval_start`.
pub fn simulate(
    strategy: &mut dyn Strategy,
    panel: &Panel,
    eval_start: NaiveDate,
    cash: f64,
    costs: &Costs,
) -> Run {
    let n = panel.symbols.len();
    let mut book = Book::new(cash, n);
    let mut w = vec![f64::NAN; n];
    let mut equity = Vec::new();
    let mut exposure = Vec::new();
    let mut started = false;
    for (t, date) in panel.dates.iter().enumerate() {
        let bars = &panel.bars[t];
        if *date >= eval_start {
            if !started {
                book.arm_all();
                started = true;
            }
            book.execute(bars, costs);
        }
        let eq = book.mark(bars);
        if started {
            equity.push((*date, eq));
            exposure.push(book.weights().iter().map(|x| x.abs()).sum::<f64>());
        }
        w.fill(f64::NAN);
        strategy.next(*date, bars, &mut w);
        book.set_targets(&w);
    }
    let avg_exposure = exposure.iter().sum::<f64>() / exposure.len().max(1) as f64;
    Run {
        equity,
        book,
        avg_exposure,
        exposure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(o: f64, c: f64) -> Option<Bar> {
        Some(Bar {
            date: NaiveDate::MIN,
            open: o,
            high: o.max(c),
            low: o.min(c),
            close: c,
            volume: 0.0,
        })
    }
    const FREE: Costs = Costs {
        commission_bps: 0.0,
        slippage_bps: 0.0,
        min_trade: 0.0,
    };

    #[test]
    fn buy_then_mark_then_close() {
        let mut b = Book::new(1000.0, 1);
        b.set_targets(&[1.0]);
        b.execute(&[bar(10.0, 11.0)], &FREE);
        assert!((b.qty[0] - 100.0).abs() < 1e-9);
        assert!((b.mark(&[bar(10.0, 11.0)]) - 1100.0).abs() < 1e-9);
        b.set_targets(&[0.0]);
        b.execute(&[bar(12.0, 12.0)], &FREE);
        assert_eq!(b.qty[0], 0.0);
        assert!((b.cash - 1200.0).abs() < 1e-9);
    }

    #[test]
    fn nan_means_hold() {
        let mut b = Book::new(1000.0, 1);
        b.set_targets(&[1.0]);
        b.execute(&[bar(10.0, 10.0)], &FREE);
        b.set_targets(&[f64::NAN]);
        b.execute(&[bar(20.0, 20.0)], &FREE);
        assert_eq!(b.trades, 1);
    }

    #[test]
    fn costs_reduce_equity() {
        let mut b = Book::new(1000.0, 1);
        b.set_targets(&[1.0]);
        b.execute(&[bar(10.0, 10.0)], &Costs::default());
        assert!(b.mark(&[bar(10.0, 10.0)]) < 1000.0);
    }

    #[test]
    fn short_profits_when_price_falls() {
        let mut b = Book::new(1000.0, 1);
        b.set_targets(&[-1.0]);
        b.execute(&[bar(10.0, 8.0)], &FREE);
        assert!((b.mark(&[bar(10.0, 8.0)]) - 1200.0).abs() < 1e-9);
    }

    #[test]
    fn gross_capped() {
        let mut b = Book::new(1000.0, 2);
        b.set_targets(&[1.0, -1.0]);
        b.execute(&[bar(10.0, 10.0), bar(10.0, 10.0)], &FREE);
        b.mark(&[bar(10.0, 10.0), bar(10.0, 10.0)]);
        assert!((b.weights().iter().map(|x| x.abs()).sum::<f64>() - 1.0).abs() < 1e-9);
    }
}
