//! Cross-sectional methods: rank the whole universe against itself and hold the
//! winners (or losers). These see every symbol at once.

use crate::data::Bar;
use crate::indicators::Window;
use crate::strategy::{Spec, Strategy};
use chrono::NaiveDate;

/// Per-symbol trailing closes shared by the cross-sectional strategies.
struct Closes {
    w: Vec<Window>,
    n: usize,
}
impl Closes {
    fn new(n: usize) -> Self {
        Closes { w: Vec::new(), n }
    }
    fn update(&mut self, bars: &[Option<Bar>]) {
        if self.w.len() != bars.len() {
            self.w.resize_with(bars.len(), || Window::new(self.n));
        }
        for (w, b) in self.w.iter_mut().zip(bars) {
            if let Some(b) = b {
                w.push(b.close);
            }
        }
    }
    /// Return from `lookback` days ago to `skip` days ago.
    fn ret(&self, i: usize, lookback: usize, skip: usize) -> Option<f64> {
        let w = &self.w[i];
        (w.len() > lookback).then(|| w.ago(skip) / w.ago(lookback) - 1.0)
    }
    fn vol(&self, i: usize, n: usize) -> Option<f64> {
        let w = &self.w[i];
        if w.len() <= n {
            return None;
        }
        let r: Vec<f64> = (0..n).map(|k| (w.ago(k) / w.ago(k + 1)).ln()).collect();
        let m = r.iter().sum::<f64>() / n as f64;
        Some((r.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt())
    }
}

#[derive(Clone, Copy)]
enum Score {
    /// Trailing return, skipping the most recent `skip` days.
    Momentum { lookback: usize, skip: usize },
    /// Negative short-term return (buy losers).
    Reversal { lookback: usize },
    /// Negative volatility (buy calm names).
    LowVol { n: usize },
    /// Negative return in excess of the cross-sectional average.
    Residual { lookback: usize },
}

struct Rank {
    closes: Closes,
    score: Score,
    top: usize,
    long_short: bool,
    /// Only hold names with positive absolute momentum (dual momentum).
    abs_filter: bool,
    every: usize,
    day: usize,
}

impl Strategy for Rank {
    fn next(&mut self, _: NaiveDate, bars: &[Option<Bar>], w: &mut [f64]) {
        self.closes.update(bars);
        self.day += 1;
        if !(self.day - 1).is_multiple_of(self.every) {
            return;
        }
        let n = bars.len();
        let raw: Vec<Option<f64>> = (0..n)
            .map(|i| match self.score {
                _ if bars[i].is_none() => None,
                Score::Momentum { lookback, skip } => self.closes.ret(i, lookback, skip),
                Score::Reversal { lookback } | Score::Residual { lookback } => {
                    self.closes.ret(i, lookback, 0).map(|r| -r)
                }
                Score::LowVol { n } => self.closes.vol(i, n).map(|v| -v),
            })
            .collect();
        let mut scored: Vec<(usize, f64)> = raw
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.filter(|x| x.is_finite()).map(|x| (i, x)))
            .collect();
        if let Score::Residual { .. } = self.score {
            let m = scored.iter().map(|x| x.1).sum::<f64>() / scored.len().max(1) as f64;
            scored.iter_mut().for_each(|x| x.1 -= m);
        }
        w.fill(0.0);
        if scored.len() < 2 * self.top.min(scored.len() / 2).max(1) {
            return;
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        let k = self.top.min(scored.len() / 2).max(1);
        let unit = if self.long_short {
            0.5 / k as f64
        } else {
            1.0 / k as f64
        };
        for &(i, s) in &scored[..k] {
            if !self.abs_filter || s > 0.0 {
                w[i] = unit;
            }
        }
        if self.long_short {
            for &(i, _) in &scored[scored.len() - k..] {
                w[i] = -unit;
            }
        }
    }
}

/// Weights proportional to 1 / volatility across all names (naive risk parity).
struct InverseVol {
    closes: Closes,
    n: usize,
    every: usize,
    day: usize,
}
impl Strategy for InverseVol {
    fn next(&mut self, _: NaiveDate, bars: &[Option<Bar>], w: &mut [f64]) {
        self.closes.update(bars);
        self.day += 1;
        if !(self.day - 1).is_multiple_of(self.every) {
            return;
        }
        let inv: Vec<f64> = (0..bars.len())
            .map(|i| match (bars[i], self.closes.vol(i, self.n)) {
                (Some(_), Some(v)) if v > 0.0 => 1.0 / v,
                _ => 0.0,
            })
            .collect();
        let tot: f64 = inv.iter().sum();
        if tot > 0.0 {
            for (wi, x) in w.iter_mut().zip(inv) {
                *wi = x / tot;
            }
        }
    }
}

/// Equal weight across listed names, rebalanced periodically.
struct EqualWeight {
    every: usize,
    day: usize,
}
impl Strategy for EqualWeight {
    fn next(&mut self, _: NaiveDate, bars: &[Option<Bar>], w: &mut [f64]) {
        self.day += 1;
        if !(self.day - 1).is_multiple_of(self.every) {
            return;
        }
        let k = bars.iter().filter(|b| b.is_some()).count().max(1) as f64;
        for (wi, b) in w.iter_mut().zip(bars) {
            *wi = if b.is_some() { 1.0 / k } else { 0.0 };
        }
    }
}

pub fn specs() -> Vec<Spec> {
    let mut v = Vec::new();
    let rank = |score, top, ls, abs, every| {
        move || Rank {
            closes: Closes::new(260),
            score,
            top,
            long_short: ls,
            abs_filter: abs,
            every,
            day: 0,
        }
    };
    for (lb, skip) in [(63, 0), (126, 0), (252, 21)] {
        for top in [3, 5, 10] {
            let s = Score::Momentum { lookback: lb, skip };
            v.push(Spec::portfolio(
                format!("cross_sectional.momentum.{lb}.{skip}.top{top}"),
                "cross_sectional",
                format!("Hold the {top} best performers over {lb} days (skipping {skip}), rebalanced monthly."),
                rank(s, top, false, false, 21),
            ));
            v.push(Spec::portfolio(
                format!("cross_sectional.dual_momentum.{lb}.{skip}.top{top}"),
                "cross_sectional",
                format!("Top {top} by {lb}-day momentum, but only those with a positive return (else cash). Monthly."),
                rank(s, top, false, true, 21),
            ));
        }
        v.push(Spec::portfolio(
            format!("cross_sectional.momentum_ls.{lb}.{skip}.top5"),
            "cross_sectional",
            format!("Market-neutral: long the top 5 and short the bottom 5 by {lb}-day momentum. Monthly."),
            rank(Score::Momentum { lookback: lb, skip }, 5, true, false, 21),
        ));
    }
    for (lb, top) in [(5, 3), (5, 5), (10, 5), (21, 5)] {
        v.push(Spec::portfolio(
            format!("cross_sectional.reversal.{lb}.top{top}"),
            "cross_sectional",
            format!("Buy the {top} worst performers of the last {lb} days, rebalanced weekly."),
            rank(Score::Reversal { lookback: lb }, top, false, false, 5),
        ));
    }
    for lb in [5, 10] {
        v.push(Spec::portfolio(
            format!("cross_sectional.residual_reversal.{lb}.ls"),
            "cross_sectional",
            format!("Stat-arb: long the 5 biggest {lb}-day underperformers vs the universe average, short the 5 biggest outperformers. Weekly."),
            rank(Score::Residual { lookback: lb }, 5, true, false, 5),
        ));
    }
    for (n, top) in [(60, 5), (60, 10), (252, 5)] {
        v.push(Spec::portfolio(
            format!("cross_sectional.low_vol.{n}.top{top}"),
            "cross_sectional",
            format!("Hold the {top} least volatile names ({n}-day), rebalanced monthly (low-volatility anomaly)."),
            rank(Score::LowVol { n }, top, false, false, 21),
        ));
    }
    for n in [20, 60] {
        v.push(Spec::portfolio(
            format!("cross_sectional.inverse_vol.{n}"),
            "cross_sectional",
            format!(
                "All names weighted by 1/vol ({n}-day), rebalanced monthly (naive risk parity)."
            ),
            move || InverseVol {
                closes: Closes::new(n + 2),
                n,
                every: 21,
                day: 0,
            },
        ));
    }
    v.push(Spec::portfolio(
        "cross_sectional.equal_weight.monthly",
        "cross_sectional",
        "Equal weight across all names, rebalanced monthly.",
        || EqualWeight { every: 21, day: 0 },
    ));
    v
}
