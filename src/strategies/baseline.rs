//! Baselines. Every other method should be judged against these:
//! buy-and-hold is the bar to beat, and the random strategies form a *null
//! distribution* - with hundreds of methods, some will look great by luck alone.

use crate::data::Bar;
use crate::rng::Rng;
use crate::strategy::{Spec, Strategy, SymbolStrategy};
use chrono::{Datelike, NaiveDate};

pub const RANDOM_PREFIX: &str = "baseline.random.";

struct BuyHold;
impl SymbolStrategy for BuyHold {
    fn next(&mut self, _: NaiveDate, _: &Bar) -> f64 {
        1.0
    }
}

/// Coin-flip long/flat per symbol, re-drawn every `hold` calendar days.
/// Decisions are a pure function of (seed, period, symbol index) so replays are exact.
struct Random {
    seed: u64,
    hold: i32,
    period: Option<u64>,
}
impl Strategy for Random {
    fn next(&mut self, date: NaiveDate, bars: &[Option<Bar>], w: &mut [f64]) {
        let period = (date.num_days_from_ce() / self.hold) as u64;
        if self.period.replace(period) == Some(period) {
            return; // hold until the next draw
        }
        let listed = bars.iter().filter(|b| b.is_some()).count().max(1) as f64;
        for (i, b) in bars.iter().enumerate() {
            let mut rng = Rng::new(self.seed.wrapping_mul(1_000_003) ^ (period << 20) ^ i as u64);
            w[i] = if b.is_some() && rng.uniform() < 0.5 {
                1.0 / listed
            } else {
                0.0
            };
        }
    }
}

pub fn specs() -> Vec<Spec> {
    let mut v = vec![Spec::per_symbol(
        "baseline.buy_hold",
        "baseline",
        "Equal-weight buy and hold of every symbol. The benchmark.",
        || BuyHold,
    )];
    for seed in 1..=20u64 {
        v.push(Spec::portfolio(
            format!("{RANDOM_PREFIX}{seed:02}"),
            "baseline",
            format!("Null model #{seed}: random long/flat per symbol, re-drawn weekly. Any strategy that can't beat most of these has no demonstrated edge."),
            move || Random { seed, hold: 7, period: None },
        ));
    }
    v
}
