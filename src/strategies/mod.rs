//! The strategy zoo. To add a method: write a `SymbolStrategy` (one asset at a
//! time) or a `Strategy` (whole universe), then register parameter variants in
//! that family's `specs()`.

pub mod baseline;
pub mod calendar;
pub mod cross_sectional;
pub mod ensemble;
pub mod learning;
pub mod mean_reversion;
pub mod momentum;
pub mod statistical;
pub mod trend;
pub mod volatility;
pub mod volume;

use crate::strategy::Spec;

/// Every registered strategy.
pub fn registry() -> Vec<Spec> {
    let mut v = Vec::new();
    v.extend(baseline::specs());
    v.extend(trend::specs());
    v.extend(momentum::specs());
    v.extend(mean_reversion::specs());
    v.extend(volatility::specs());
    v.extend(volume::specs());
    v.extend(statistical::specs());
    v.extend(learning::specs());
    v.extend(calendar::specs());
    v.extend(cross_sectional::specs());
    let ens = ensemble::specs(&v);
    v.extend(ens);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{synthetic, Panel};
    use crate::engine::{simulate, Costs};
    use chrono::NaiveDate;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique() {
        let r = registry();
        let ids: HashSet<_> = r.iter().map(|s| s.id.clone()).collect();
        assert_eq!(ids.len(), r.len());
        assert!(r.len() >= 300, "only {} strategies", r.len());
    }

    /// Every strategy runs, stays solvent, and is deterministic.
    #[test]
    fn all_strategies_run_deterministically() {
        let syms: Vec<String> = ["AAA", "BBB", "CCC", "DDD", "EEE", "FFF", "GGG", "HHH"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let d = |y| NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
        let panel = Panel::new(synthetic::generate(&syms, d(2016), d(2020), 7), d(2016));
        for spec in registry() {
            let a = simulate(
                (spec.build)().as_mut(),
                &panel,
                d(2018),
                1e5,
                &Costs::default(),
            );
            let b = simulate(
                (spec.build)().as_mut(),
                &panel,
                d(2018),
                1e5,
                &Costs::default(),
            );
            let (ea, eb) = (a.equity.last().unwrap().1, b.equity.last().unwrap().1);
            assert!(ea.is_finite() && ea > 0.0, "{}: equity {ea}", spec.id);
            assert_eq!(ea, eb, "{} is not deterministic", spec.id);
            assert!(
                a.avg_exposure <= 1.05,
                "{} over-leveraged: {}",
                spec.id,
                a.avg_exposure
            );
        }
    }

    /// On a pure random walk there is nothing to find. A strategy that peeks at
    /// future bars would show an implausibly high Sharpe here.
    #[test]
    fn no_edge_on_random_walk() {
        use crate::data::{Bar, Series};
        use crate::rng::Rng;
        let d0 = NaiveDate::from_ymd_opt(2010, 1, 4).unwrap();
        let series: Vec<Series> = (0..6)
            .map(|k| {
                let mut rng = Rng::new(100 + k);
                let mut px = 100.0;
                let bars = (0..2500)
                    .map(|t| {
                        let open = px * (1.0 + 0.003 * rng.normal());
                        let close = open * (1.0 + 0.01 * rng.normal());
                        let (hi, lo) = (
                            open.max(close) * (1.0 + 0.004 * rng.uniform()),
                            open.min(close) * (1.0 - 0.004 * rng.uniform()),
                        );
                        px = close;
                        Bar {
                            date: d0 + chrono::Duration::days(t),
                            open,
                            high: hi,
                            low: lo,
                            close,
                            volume: 1e6 * (1.0 + rng.uniform()),
                        }
                    })
                    .collect();
                Series {
                    symbol: format!("S{k}"),
                    bars,
                }
            })
            .collect();
        let panel = Panel::new(series, d0);
        let eval = panel.dates[300];
        let free = Costs {
            commission_bps: 0.0,
            slippage_bps: 0.0,
            min_trade: 0.0,
        };
        for spec in registry() {
            let run = simulate((spec.build)().as_mut(), &panel, eval, 1e5, &free);
            let m = crate::metrics::compute(&run.equity);
            assert!(
                m.sharpe < 1.5,
                "{} has Sharpe {:.2} on pure noise: lookahead?",
                spec.id,
                m.sharpe
            );
        }
    }
}
