//! Synthetic market generator for offline development and tests.
//!
//! Prices follow a regime-switching GBM (calm/trending vs. volatile/mean-reverting)
//! with a shared market factor, so strategies have *something* to find, but nothing
//! here should be mistaken for evidence about real markets.

use super::{Bar, Series};
use crate::rng::{hash_str, Rng};
use chrono::{Datelike, Duration, NaiveDate, Weekday};

pub fn generate(symbols: &[String], start: NaiveDate, end: NaiveDate, seed: u64) -> Vec<Series> {
    let mut days = Vec::new();
    let mut d = start;
    while d <= end {
        if !matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
            days.push(d);
        }
        d += Duration::days(1);
    }
    // Shared market factor.
    let mut mrng = Rng::new(seed);
    let mut regime_bull = true;
    let market: Vec<f64> = days
        .iter()
        .map(|_| {
            if mrng.uniform() < 0.004 {
                regime_bull = !regime_bull;
            }
            let (mu, sigma) = if regime_bull {
                (0.0005, 0.008)
            } else {
                (-0.0003, 0.02)
            };
            mu + sigma * mrng.normal()
        })
        .collect();

    symbols
        .iter()
        .map(|sym| {
            let mut rng = Rng::new(seed ^ hash_str(sym));
            let beta = 0.5 + rng.uniform();
            let idio = 0.005 + 0.015 * rng.uniform();
            let ar = -0.15 + 0.3 * rng.uniform(); // mild autocorrelation, symbol specific
            let mut price = 20.0 + 180.0 * rng.uniform();
            let mut prev_ret = 0.0;
            let bars = days
                .iter()
                .zip(&market)
                .map(|(&date, &m)| {
                    let ret = beta * m + ar * prev_ret + idio * rng.normal();
                    prev_ret = ret;
                    let open = price * (1.0 + 0.3 * idio * rng.normal());
                    let close = (price * ret.exp()).max(0.01);
                    let span = (open - close).abs() + price * idio * rng.uniform();
                    let high = open.max(close) + 0.5 * span * rng.uniform();
                    let low = (open.min(close) - 0.5 * span * rng.uniform()).max(0.005);
                    let volume = (1e6 * (1.0 + 20.0 * ret.abs()) * (0.5 + rng.uniform())).round();
                    price = close;
                    Bar {
                        date,
                        open: open.max(0.01),
                        high,
                        low,
                        close,
                        volume,
                    }
                })
                .collect();
            Series {
                symbol: sym.clone(),
                bars,
            }
        })
        .collect()
}
