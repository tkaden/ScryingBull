//! Run every strategy over history, in parallel, and score it.

use crate::data::Panel;
use crate::engine::{simulate, Costs};
use crate::metrics::{compute, Metrics};
use crate::strategies::baseline::RANDOM_PREFIX;
use crate::strategy::Spec;
use chrono::{Datelike, NaiveDate};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BacktestResult {
    pub id: String,
    pub family: String,
    pub description: String,
    pub metrics: Metrics,
    /// Metrics over the most recent 252 trading days.
    pub last_year: Metrics,
    pub trades: u64,
    /// Average annual turnover (traded notional / equity).
    pub turnover: f64,
    pub avg_exposure: f64,
    /// Share of the random null strategies this one beats on Sharpe (0..1).
    pub beats_random: f64,
    /// Weekly equity samples (date, equity) for charts.
    pub curve: Vec<(NaiveDate, f64)>,
}

pub fn run_all(
    specs: &[Spec],
    panel: &Panel,
    eval_start: NaiveDate,
    cash: f64,
    costs: &Costs,
) -> Vec<BacktestResult> {
    let mut results: Vec<BacktestResult> = specs
        .par_iter()
        .map(|spec| {
            let mut s = (spec.build)();
            let run = simulate(s.as_mut(), panel, eval_start, cash, costs);
            let years = (run.equity.len() as f64 / 252.0).max(1e-9);
            let tail = &run.equity[run.equity.len().saturating_sub(253)..];
            BacktestResult {
                id: spec.id.clone(),
                family: spec.family.to_string(),
                description: spec.description.clone(),
                metrics: compute(&run.equity),
                last_year: compute(tail),
                trades: run.book.trades,
                turnover: run.book.turnover / years,
                avg_exposure: run.avg_exposure,
                beats_random: 0.0,
                curve: weekly(&run.equity),
            }
        })
        .collect();
    let mut null: Vec<f64> = results
        .iter()
        .filter(|r| r.id.starts_with(RANDOM_PREFIX))
        .map(|r| r.metrics.sharpe)
        .collect();
    null.sort_by(f64::total_cmp);
    if !null.is_empty() {
        for r in &mut results {
            r.beats_random =
                null.iter().filter(|s| **s < r.metrics.sharpe).count() as f64 / null.len() as f64;
        }
    }
    results.sort_by(|a, b| b.metrics.sharpe.total_cmp(&a.metrics.sharpe));
    results
}

/// Keep the last observation of each ISO week (plus the final point).
pub fn weekly(eq: &[(NaiveDate, f64)]) -> Vec<(NaiveDate, f64)> {
    let mut out: Vec<(NaiveDate, f64)> = Vec::new();
    for (k, &(d, v)) in eq.iter().enumerate() {
        let last_of_week = eq
            .get(k + 1)
            .is_none_or(|(n, _)| n.iso_week() != d.iso_week());
        if last_of_week {
            out.push((d, (v * 100.0).round() / 100.0));
        }
    }
    out
}
