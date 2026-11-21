//! Performance statistics from an equity curve.

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const DAYS: f64 = 252.0;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    pub days: usize,
    pub total_return: f64,
    pub cagr: f64,
    pub vol: f64,
    pub sharpe: f64,
    pub sortino: f64,
    pub max_drawdown: f64,
    pub calmar: f64,
    /// Fraction of days with a positive return.
    pub hit_rate: f64,
    pub yearly: BTreeMap<i32, f64>,
}

pub fn compute(eq: &[(NaiveDate, f64)]) -> Metrics {
    if eq.len() < 2 {
        return Metrics {
            days: eq.len(),
            start: eq.first().map(|x| x.0),
            end: eq.last().map(|x| x.0),
            ..Default::default()
        };
    }
    let rets: Vec<f64> = eq.windows(2).map(|w| w[1].1 / w[0].1 - 1.0).collect();
    let n = rets.len() as f64;
    let mean = rets.iter().sum::<f64>() / n;
    let sd = (rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0)).sqrt();
    let dsd = (rets.iter().map(|r| r.min(0.0).powi(2)).sum::<f64>() / n).sqrt();
    let total = eq.last().unwrap().1 / eq[0].1 - 1.0;
    let years = n / DAYS;
    let cagr = if total > -1.0 {
        (1.0 + total).powf(1.0 / years) - 1.0
    } else {
        -1.0
    };
    let mut peak = f64::MIN;
    let mut mdd = 0.0f64;
    for (_, v) in eq {
        peak = peak.max(*v);
        mdd = mdd.min(v / peak - 1.0);
    }
    let mut yearly = BTreeMap::new();
    let mut year_start = eq[0].1;
    for w in eq.windows(2) {
        if w[1].0.year() != w[0].0.year() {
            yearly.insert(w[0].0.year(), w[0].1 / year_start - 1.0);
            year_start = w[0].1;
        }
    }
    let (ly, lv) = eq.last().map(|(d, v)| (d.year(), *v)).unwrap();
    yearly.insert(ly, lv / year_start - 1.0);
    let ann = DAYS.sqrt();
    Metrics {
        start: Some(eq[0].0),
        end: Some(eq.last().unwrap().0),
        days: eq.len(),
        total_return: total,
        cagr,
        vol: sd * ann,
        sharpe: if sd > 0.0 { mean / sd * ann } else { 0.0 },
        sortino: if dsd > 0.0 { mean / dsd * ann } else { 0.0 },
        max_drawdown: mdd,
        calmar: if mdd < 0.0 { cagr / -mdd } else { 0.0 },
        hit_rate: rets.iter().filter(|r| **r > 0.0).count() as f64 / n,
        yearly,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawdown_and_return() {
        let d = |k| NaiveDate::from_ymd_opt(2024, 1, k).unwrap();
        let m = compute(&[(d(1), 100.0), (d(2), 120.0), (d(3), 90.0), (d(4), 110.0)]);
        assert!((m.total_return - 0.1).abs() < 1e-12);
        assert!((m.max_drawdown + 0.25).abs() < 1e-12);
        assert_eq!(m.yearly.len(), 1);
    }
}
