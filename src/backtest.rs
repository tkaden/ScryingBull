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
    /// Whole evaluation window.
    pub metrics: Metrics,
    /// In-sample period [eval_start, validation_start).
    pub train: Metrics,
    /// Held-out period [validation_start, end].
    pub validation: Metrics,
    /// Metrics over the most recent 252 trading days.
    pub last_year: Metrics,
    pub trades: u64,
    /// Average annual turnover (traded notional / equity).
    pub turnover: f64,
    pub avg_exposure: f64,
    /// Share of the random null strategies this one beats on train-period Sharpe (0..1).
    pub beats_random_train: f64,
    /// Same, on the validation period.
    pub beats_random_validation: f64,
    /// Weekly equity samples (date, equity) for charts.
    pub curve: Vec<(NaiveDate, f64)>,
}

/// One (date, equity) observation.
pub type Point = (NaiveDate, f64);

/// Split an equity curve at `at`. The validation half starts from the last
/// train-period close, so its first day's return is included.
pub fn split(eq: &[Point], at: NaiveDate) -> (&[Point], &[Point]) {
    let k = eq.partition_point(|(d, _)| *d < at);
    (&eq[..k], &eq[k.saturating_sub(1)..])
}

pub fn run_all(
    specs: &[Spec],
    panel: &Panel,
    eval_start: NaiveDate,
    validation_start: NaiveDate,
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
            let (train, validation) = split(&run.equity, validation_start);
            BacktestResult {
                id: spec.id.clone(),
                family: spec.family.to_string(),
                description: spec.description.clone(),
                metrics: compute(&run.equity),
                train: compute(train),
                validation: compute(validation),
                last_year: compute(tail),
                trades: run.book.trades,
                turnover: run.book.turnover / years,
                avg_exposure: run.avg_exposure,
                beats_random_train: 0.0,
                beats_random_validation: 0.0,
                curve: weekly(&run.equity),
            }
        })
        .collect();
    let null = |f: fn(&BacktestResult) -> f64, rs: &[BacktestResult]| -> Vec<f64> {
        rs.iter()
            .filter(|r| r.id.starts_with(RANDOM_PREFIX))
            .map(f)
            .collect()
    };
    let null_train = null(|r| r.train.sharpe, &results);
    let null_val = null(|r| r.validation.sharpe, &results);
    let beats = |x: f64, null: &[f64]| {
        null.iter().filter(|s| **s < x).count() as f64 / null.len().max(1) as f64
    };
    for r in &mut results {
        r.beats_random_train = beats(r.train.sharpe, &null_train);
        r.beats_random_validation = beats(r.validation.sharpe, &null_val);
    }
    // Rank by the train period: that is what you would have chosen with.
    results.sort_by(|a, b| b.train.sharpe.total_cmp(&a.train.sharpe));
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

/// How well does the train period predict the validation period?
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidationReport {
    pub strategies: usize,
    /// Spearman rank correlation of train vs validation Sharpe (non-random strategies).
    pub rank_corr: f64,
    /// Mean validation Sharpe of the top 10 strategies by train Sharpe.
    pub top10_val_sharpe: f64,
    /// Median validation Sharpe over all non-random strategies.
    pub median_val_sharpe: f64,
    pub buy_hold_val_sharpe: Option<f64>,
    /// Non-random strategies beating 95% of random baselines in both periods.
    pub robust: Vec<String>,
}

fn ranks(x: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..x.len()).collect();
    idx.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    let mut r = vec![0.0; x.len()];
    for (rank, &i) in idx.iter().enumerate() {
        r[i] = rank as f64;
    }
    r
}

pub fn validation_report(results: &[BacktestResult]) -> ValidationReport {
    let real: Vec<&BacktestResult> = results
        .iter()
        .filter(|r| !r.id.starts_with(RANDOM_PREFIX))
        .collect();
    let tr: Vec<f64> = real.iter().map(|r| r.train.sharpe).collect();
    let va: Vec<f64> = real.iter().map(|r| r.validation.sharpe).collect();
    let (rt, rv) = (ranks(&tr), ranks(&va));
    let n = rt.len() as f64;
    let m = (n - 1.0) / 2.0;
    let cov: f64 = rt.iter().zip(&rv).map(|(a, b)| (a - m) * (b - m)).sum();
    let var: f64 = rt.iter().map(|a| (a - m).powi(2)).sum();
    let mut by_train = real.clone();
    by_train.sort_by(|a, b| b.train.sharpe.total_cmp(&a.train.sharpe));
    let top: Vec<f64> = by_train
        .iter()
        .take(10)
        .map(|r| r.validation.sharpe)
        .collect();
    let mut sorted = va.clone();
    sorted.sort_by(f64::total_cmp);
    ValidationReport {
        strategies: real.len(),
        rank_corr: if var > 0.0 { cov / var } else { 0.0 },
        top10_val_sharpe: top.iter().sum::<f64>() / top.len().max(1) as f64,
        median_val_sharpe: sorted.get(sorted.len() / 2).copied().unwrap_or(0.0),
        buy_hold_val_sharpe: results
            .iter()
            .find(|r| r.id == "baseline.buy_hold")
            .map(|r| r.validation.sharpe),
        robust: by_train
            .iter()
            .filter(|r| r.beats_random_train >= 0.95 && r.beats_random_validation >= 0.95)
            .map(|r| r.id.clone())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_overlaps_by_one_point() {
        let d = |k| NaiveDate::from_ymd_opt(2024, 1, k).unwrap();
        let eq = [(d(1), 1.0), (d(2), 2.0), (d(3), 3.0), (d(4), 4.0)];
        let (a, b) = split(&eq, d(3));
        assert_eq!(a.len(), 2);
        assert_eq!(b[0].0, d(2));
        assert_eq!(b.len(), 3);
    }

    #[test]
    fn rank_corr_is_one_for_monotone() {
        assert_eq!(ranks(&[3.0, 1.0, 2.0]), vec![2.0, 0.0, 1.0]);
    }
}
