//! Volatility-aware methods: position sizing and regime filters based on risk.

use crate::data::Bar;
use crate::indicators::{Atr, Ema, RetVol, Window};
use crate::strategy::{Spec, SymbolStrategy};
use chrono::NaiveDate;

struct VolTarget {
    vol: RetVol,
    target: f64,
}
impl SymbolStrategy for VolTarget {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        match self.vol.update(b.close) {
            Some(sd) if sd > 0.0 => (self.target / (sd * 252f64.sqrt())).min(1.0),
            _ => 0.0,
        }
    }
}

/// Long only while short-term volatility is below long-term volatility.
struct VolRegime {
    short: RetVol,
    long: RetVol,
}
impl SymbolStrategy for VolRegime {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        match (self.short.update(b.close), self.long.update(b.close)) {
            (Some(s), Some(l)) if s < l => 1.0,
            _ => 0.0,
        }
    }
}

struct Keltner {
    ema: Ema,
    atr: Atr,
    mult: f64,
    pos: f64,
}
impl SymbolStrategy for Keltner {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let (Some(m), Some(a)) = (
            self.ema.update(b.close),
            self.atr.update(b.high, b.low, b.close),
        ) else {
            return 0.0;
        };
        if b.close > m + self.mult * a {
            self.pos = 1.0;
        } else if b.close < m {
            self.pos = 0.0;
        }
        self.pos
    }
}

/// Chandelier exit: stay long until close falls `mult` ATRs below the highest close;
/// re-enter on a fresh `reentry`-day high.
struct Chandelier {
    atr: Atr,
    mult: f64,
    highs: Window,
    peak: f64,
    pos: f64,
}
impl SymbolStrategy for Chandelier {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let new_high = self.highs.full() && b.close > self.highs.max();
        self.highs.push(b.close);
        let Some(a) = self.atr.update(b.high, b.low, b.close) else {
            return 0.0;
        };
        if self.pos == 0.0 && (new_high || self.peak == 0.0) {
            self.pos = 1.0;
            self.peak = b.close;
        }
        if self.pos > 0.0 {
            self.peak = self.peak.max(b.close);
            if b.close < self.peak - self.mult * a {
                self.pos = 0.0;
            }
        }
        self.pos
    }
}

/// Narrow-range day (NR-n): volatility contraction often precedes expansion.
/// Go long the day after an NR-n day that closed in the upper half; hold `hold` days.
struct NarrowRange {
    ranges: Window,
    hold: usize,
    left: usize,
}
impl SymbolStrategy for NarrowRange {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let r = b.high - b.low;
        let nr = self.ranges.full() && r < self.ranges.min();
        self.ranges.push(r);
        self.left = self.left.saturating_sub(1);
        if nr && b.close > (b.high + b.low) / 2.0 {
            self.left = self.hold;
        }
        if self.left > 0 {
            1.0
        } else {
            0.0
        }
    }
}

pub fn specs() -> Vec<Spec> {
    let mut v = Vec::new();
    for n in [20, 60] {
        for t in [0.10, 0.15, 0.20] {
            v.push(Spec::per_symbol(
                format!("volatility.vol_target.{n}.{t}"),
                "volatility",
                format!(
                    "Always long, scaled to {:.0}% annualised vol using {n}-day realised vol.",
                    t * 100.0
                ),
                move || VolTarget {
                    vol: RetVol::new(n),
                    target: t,
                },
            ));
        }
    }
    for (s, l) in [(10, 60), (20, 120), (5, 50)] {
        v.push(Spec::per_symbol(
            format!("volatility.vol_regime.{s}.{l}"),
            "volatility",
            format!("Long while {s}-day vol < {l}-day vol (calm regime), else cash."),
            move || VolRegime {
                short: RetVol::new(s),
                long: RetVol::new(l),
            },
        ));
    }
    for (n, m) in [(20, 1.0), (20, 2.0), (50, 1.5), (50, 2.5)] {
        v.push(Spec::per_symbol(
            format!("volatility.keltner.{n}.{m}"),
            "volatility",
            format!("Long on close above EMA({n}) + {m} ATR, exit below the EMA."),
            move || Keltner {
                ema: Ema::new(n),
                atr: Atr::new(n),
                mult: m,
                pos: 0.0,
            },
        ));
    }
    for m in [2.0, 3.0, 4.0] {
        v.push(Spec::per_symbol(
            format!("volatility.chandelier.22.{m}"),
            "volatility",
            format!("Long with a {m}x ATR(22) trailing stop; re-enter on a 50-day closing high."),
            move || Chandelier {
                atr: Atr::new(22),
                mult: m,
                highs: Window::new(50),
                peak: 0.0,
                pos: 0.0,
            },
        ));
    }
    for (n, h) in [(4, 3), (7, 3), (7, 5)] {
        v.push(Spec::per_symbol(
            format!("volatility.nr{n}.{h}"),
            "volatility",
            format!(
                "After an NR{n} day (narrowest range in {n}) closing strong, go long for {h} days."
            ),
            move || NarrowRange {
                ranges: Window::new(n - 1),
                hold: h,
                left: 0,
            },
        ));
    }
    v
}
