//! Value investing research: which stocks are cheap, and does cheapness pay?
//!
//! At every month end, each stock is valued with five yields (higher = cheaper):
//!
//! | measure          | definition                                      |
//! |------------------|-------------------------------------------------|
//! | earnings yield   | annual net income / market cap                  |
//! | book-to-market   | shareholders' equity / market cap               |
//! | sales yield      | annual revenue / market cap                     |
//! | free-cash yield  | (operating cash flow - capex) / market cap      |
//! | EBIT / EV        | operating income / (market cap + debt - cash)   |
//!
//! The composite value score is the average cross-sectional percentile of the
//! yields a stock has (at least three). A Piotroski F-score (0-9) measures
//! financial health, to separate cheap-and-improving stocks from value traps.
//!
//! Everything is point-in-time: fundamentals count only once filed with the SEC,
//! prices are the close on the scoring day, and portfolios trade at the next open.
//! The predictive power of each signal is measured month by month with the
//! information coefficient (rank correlation between score and the next month's return).

use crate::backtest::{self, BacktestResult};
use crate::data::{yahoo::Extras, Bar, Panel};
use crate::engine::Costs;
use crate::fundamentals::Facts;
use crate::strategy::{Spec, Strategy};
use chrono::{Datelike, NaiveDate};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The individual value signals, in the order they appear in [`Snapshot::yields`].
pub const SIGNALS: [(&str, &str); 5] = [
    ("earnings_yield", "Earnings yield"),
    ("book_to_market", "Book-to-market"),
    ("sales_yield", "Sales yield"),
    ("fcf_yield", "Free-cash-flow yield"),
    ("ebit_ev", "EBIT / EV"),
];

/// One stock's valuation at one date.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub price: f64,
    pub market_cap: f64,
    /// Yields per [`SIGNALS`] (None when an input is missing).
    pub yields: [Option<f64>; 5],
    /// Cross-sectional percentile of each yield, 1 = cheapest.
    pub ranks: [Option<f64>; 5],
    /// Composite value percentile, 1 = cheapest.
    pub value: Option<f64>,
    /// Piotroski F-score, 0-9.
    pub fscore: Option<u8>,
    /// 12-month return skipping the last month.
    pub momentum: Option<f64>,
    /// Filing date of the newest annual report used.
    pub report_filed: Option<NaiveDate>,
}

pub struct ScoreTable {
    /// Month-end dates on which portfolios rebalance.
    pub rebalance: Vec<NaiveDate>,
    /// Snapshots per date (rebalance dates plus the latest date), indexed like the panel's symbols.
    pub by_date: BTreeMap<NaiveDate, Vec<Option<Snapshot>>>,
}

fn price_on(extras: &Extras, t: NaiveDate) -> Option<f64> {
    let i = extras.raw_close.partition_point(|(d, _)| *d <= t);
    let (d, p) = *extras.raw_close.get(i.checked_sub(1)?)?;
    ((t - d).num_days() <= 7).then_some(p)
}

/// Shares reported at `asof` restated to today's split basis.
fn split_adjust(shares: f64, asof: NaiveDate, splits: &[(NaiveDate, f64)]) -> f64 {
    shares
        * splits
            .iter()
            .filter(|(d, _)| *d > asof)
            .map(|(_, r)| r)
            .product::<f64>()
}

fn piotroski(f: &Facts, t: NaiveDate) -> Option<(u8, NaiveDate)> {
    let ni = f.as_of("net_income", t, 456)?;
    let at = |field: &str, end: NaiveDate| {
        f.fields
            .get(field)?
            .iter()
            .filter(|x| x.filed <= t && x.end == end)
            .max_by_key(|x| x.filed)
            .map(|x| x.val)
    };
    let ni_p = f.prior_year("net_income", ni, t)?;
    let (e, ep) = (ni.end, ni_p.end);
    let flow = |field: &str, end: NaiveDate| {
        f.fields
            .get(field)?
            .iter()
            .filter(|x| x.filed <= t && x.end == end)
            .max_by_key(|x| x.filed)
            .map(|x| x.val)
    };
    let (a, ap) = (at("assets", e)?, at("assets", ep)?);
    if a <= 0.0 || ap <= 0.0 {
        return None;
    }
    let mut tests: Vec<Option<bool>> = Vec::new();
    let roa = ni.val / a;
    let roa_p = ni_p.val / ap;
    let cfo = flow("cfo", e);
    tests.push(Some(roa > 0.0));
    tests.push(cfo.map(|c| c > 0.0));
    tests.push(Some(roa > roa_p));
    tests.push(cfo.map(|c| c > ni.val));
    let lev = |end, assets: f64| Some(at("lt_debt", end).unwrap_or(0.0) / assets);
    tests.push(lev(e, a).zip(lev(ep, ap)).map(|(x, y)| x < y));
    let cr = |end| {
        Some(at("assets_current", end)? / at("liabilities_current", end).filter(|x| *x > 0.0)?)
    };
    tests.push(cr(e).zip(cr(ep)).map(|(x, y)| x > y));
    tests.push(
        flow("diluted_shares", e)
            .zip(flow("diluted_shares", ep))
            .map(|(x, y)| x <= y * 1.001),
    );
    let gm = |end| Some(flow("gross_profit", end)? / flow("revenue", end).filter(|x| *x > 0.0)?);
    tests.push(gm(e).zip(gm(ep)).map(|(x, y)| x > y));
    let turn = |end, assets: f64| Some(flow("revenue", end)? / assets);
    tests.push(turn(e, a).zip(turn(ep, ap)).map(|(x, y)| x > y));
    let avail: Vec<bool> = tests.into_iter().flatten().collect();
    if avail.len() < 6 {
        return None;
    }
    let passed = avail.iter().filter(|x| **x).count() as f64;
    Some(((passed / avail.len() as f64 * 9.0).round() as u8, ni.filed))
}

fn snapshot(f: &Facts, ex: &Extras, t: NaiveDate) -> Option<Snapshot> {
    let price = price_on(ex, t)?;
    let shares = f
        .as_of("shares", t, 200)
        .or_else(|| f.as_of("diluted_shares", t, 456))
        .map(|s| split_adjust(s.val, s.end, &ex.splits))?;
    let mcap = shares * price;
    if !(mcap.is_finite() && mcap > 1e7) {
        return None;
    }
    let flow = |field| f.as_of(field, t, 456).map(|x| x.val);
    let inst = |field| f.as_of(field, t, 200).map(|x| x.val);
    let ni = flow("net_income");
    let ev = mcap + inst("lt_debt").unwrap_or(0.0) - inst("cash").unwrap_or(0.0);
    let yields = [
        ni.map(|x| x / mcap),
        inst("equity").map(|x| x / mcap),
        flow("revenue").filter(|x| *x > 0.0).map(|x| x / mcap),
        flow("cfo").zip(flow("capex")).map(|(c, k)| (c - k) / mcap),
        flow("op_income").filter(|_| ev > 0.0).map(|x| x / ev),
    ];
    // Guard against unit or share-count errors in filings.
    if yields
        .iter()
        .flatten()
        .any(|y| !y.is_finite() || y.abs() > 20.0)
    {
        return None;
    }
    let (fscore, filed) = match piotroski(f, t) {
        Some((s, d)) => (Some(s), Some(d)),
        None => (None, f.as_of("net_income", t, 456).map(|x| x.filed)),
    };
    Some(Snapshot {
        price,
        market_cap: mcap,
        yields,
        fscore,
        report_filed: filed,
        ..Default::default()
    })
}

/// Percentile ranks in (0, 1]; larger values rank higher. Ties share the average rank.
fn pct_ranks(vals: &[Option<f64>]) -> Vec<Option<f64>> {
    let mut idx: Vec<(usize, f64)> = vals
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|x| (i, x)))
        .collect();
    idx.sort_by(|a, b| a.1.total_cmp(&b.1));
    let n = idx.len() as f64;
    let mut out = vec![None; vals.len()];
    let mut k = 0;
    while k < idx.len() {
        let mut j = k;
        while j + 1 < idx.len() && idx[j + 1].1 == idx[k].1 {
            j += 1;
        }
        let r = ((k + j) as f64 / 2.0 + 1.0) / n;
        for m in &idx[k..=j] {
            out[m.0] = Some(r);
        }
        k = j + 1;
    }
    out
}

fn month_ends(panel: &Panel, from: NaiveDate) -> Vec<usize> {
    (0..panel.len())
        .filter(|&t| {
            panel.dates[t] >= from
                && panel
                    .dates
                    .get(t + 1)
                    .is_none_or(|n| n.month() != panel.dates[t].month())
        })
        .collect()
}

pub fn build_table(
    panel: &Panel,
    facts: &[Option<Facts>],
    extras: &[Extras],
    from: NaiveDate,
) -> ScoreTable {
    let mut dates = month_ends(panel, from);
    let last = panel.len() - 1;
    let rebalance: Vec<NaiveDate> = dates
        .iter()
        .filter(|t| **t != last)
        .map(|t| panel.dates[*t])
        .collect();
    if dates.last() != Some(&last) {
        dates.push(last);
    }
    let mut by_date = BTreeMap::new();
    for t in dates {
        let date = panel.dates[t];
        let mut snaps: Vec<Option<Snapshot>> = (0..panel.symbols.len())
            .map(|i| {
                panel.bars[t][i]?;
                let mut s = snapshot(facts[i].as_ref()?, &extras[i], date)?;
                // 12-1 momentum from total-return prices.
                if t >= 252 {
                    if let (Some(a), Some(b)) = (
                        panel.close_at_or_before(t - 21, i),
                        panel.close_at_or_before(t - 252, i),
                    ) {
                        s.momentum = Some(a / b - 1.0);
                    }
                }
                Some(s)
            })
            .collect();
        for k in 0..5 {
            let r = pct_ranks(
                &snaps
                    .iter()
                    .map(|s| s.as_ref().and_then(|s| s.yields[k]))
                    .collect::<Vec<_>>(),
            );
            for (s, r) in snaps.iter_mut().zip(r) {
                if let Some(s) = s {
                    s.ranks[k] = r;
                }
            }
        }
        for s in snaps.iter_mut().flatten() {
            let have: Vec<f64> = s.ranks.iter().flatten().copied().collect();
            if have.len() >= 3 {
                s.value = Some(have.iter().sum::<f64>() / have.len() as f64);
            }
        }
        by_date.insert(date, snaps);
    }
    ScoreTable { rebalance, by_date }
}

// ---------- portfolios ----------

type Selector = dyn Fn(&[Option<Snapshot>]) -> Vec<f64> + Send + Sync;

struct Picker {
    table: Arc<ScoreTable>,
    select: Arc<Selector>,
}

impl Strategy for Picker {
    fn next(&mut self, date: NaiveDate, _bars: &[Option<Bar>], w: &mut [f64]) {
        if self.table.rebalance.binary_search(&date).is_err() {
            return;
        }
        if let Some(snaps) = self.table.by_date.get(&date) {
            let ws = (self.select)(snaps);
            w.copy_from_slice(&ws);
        }
    }
}

fn score_of(s: &Snapshot, signal: Option<usize>) -> Option<f64> {
    match signal {
        None => s.value,
        Some(k) => s.ranks[k],
    }
}

/// Indices sorted cheapest first by `signal` (None = composite).
fn ranked(
    snaps: &[Option<Snapshot>],
    signal: Option<usize>,
    keep: impl Fn(&Snapshot) -> bool,
) -> Vec<usize> {
    let mut v: Vec<(usize, f64)> = snaps
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            s.as_ref()
                .filter(|s| keep(s))
                .and_then(|s| score_of(s, signal))
                .map(|x| (i, x))
        })
        .collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v.into_iter().map(|x| x.0).collect()
}

fn equal(n: usize, pick: &[usize]) -> Vec<f64> {
    let mut w = vec![0.0; n];
    for &i in pick {
        w[i] = 1.0 / pick.len().max(1) as f64;
    }
    w
}

fn quintile(snaps: &[Option<Snapshot>], signal: Option<usize>, q: usize) -> Vec<usize> {
    let r = ranked(snaps, signal, |_| true);
    let k = r.len() / 5;
    if k == 0 {
        return vec![];
    }
    let end = if q == 4 { r.len() } else { (q + 1) * k };
    r[q * k..end].to_vec()
}

pub fn specs(table: Arc<ScoreTable>) -> Vec<Spec> {
    let mut v = Vec::new();
    let mut add = |id: String, desc: String, sel: Arc<Selector>| {
        let t = table.clone();
        v.push(Spec::portfolio(id, "value", desc, move || Picker {
            table: t.clone(),
            select: sel.clone(),
        }));
    };
    for q in 0..5 {
        add(
            format!("value.composite.q{}", q + 1),
            format!(
                "Quintile {} of 5 by composite value score ({}), equal weight, monthly.",
                q + 1,
                if q == 0 {
                    "cheapest"
                } else if q == 4 {
                    "most expensive"
                } else {
                    "middle"
                }
            ),
            Arc::new(move |s: &[Option<Snapshot>]| equal(s.len(), &quintile(s, None, q))),
        );
    }
    for (k, (key, label)) in SIGNALS.iter().enumerate() {
        add(
            format!("value.{key}.q1"),
            format!(
                "Cheapest quintile by {} alone, equal weight, monthly.",
                label.to_lowercase()
            ),
            Arc::new(move |s: &[Option<Snapshot>]| equal(s.len(), &quintile(s, Some(k), 0))),
        );
    }
    add(
        "value.top30".into(),
        "The 30 cheapest stocks by composite value score, equal weight, monthly.".into(),
        Arc::new(|s: &[Option<Snapshot>]| {
            equal(
                s.len(),
                &ranked(s, None, |_| true)
                    .into_iter()
                    .take(30)
                    .collect::<Vec<_>>(),
            )
        }),
    );
    add(
        "value.quality_value".into(),
        "The 30 cheapest stocks among those with a Piotroski F-score of 6 or more (avoids value traps). Monthly.".into(),
        Arc::new(|s: &[Option<Snapshot>]| {
            equal(s.len(), &ranked(s, None, |x| x.fscore.is_some_and(|f| f >= 6)).into_iter().take(30).collect::<Vec<_>>())
        }),
    );
    add(
        "value.value_momentum".into(),
        "The 30 stocks with the best average of value rank and 12-1 momentum rank. Monthly.".into(),
        Arc::new(|s: &[Option<Snapshot>]| {
            let mom = pct_ranks(
                &s.iter()
                    .map(|x| x.as_ref().and_then(|x| x.momentum))
                    .collect::<Vec<_>>(),
            );
            let mut v: Vec<(usize, f64)> = s
                .iter()
                .enumerate()
                .filter_map(|(i, x)| Some((i, (x.as_ref()?.value? + mom[i]?) / 2.0)))
                .collect();
            v.sort_by(|a, b| b.1.total_cmp(&a.1));
            equal(
                s.len(),
                &v.into_iter().take(30).map(|x| x.0).collect::<Vec<_>>(),
            )
        }),
    );
    add(
        "value.long_short".into(),
        "Market-neutral: long the cheapest quintile, short the most expensive, 50% each side. Monthly.".into(),
        Arc::new(|s: &[Option<Snapshot>]| {
            let mut w = vec![0.0; s.len()];
            let (lo, hi) = (quintile(s, None, 0), quintile(s, None, 4));
            for &i in &lo {
                w[i] = 0.5 / lo.len().max(1) as f64;
            }
            for &i in &hi {
                w[i] = -0.5 / hi.len().max(1) as f64;
            }
            w
        }),
    );
    add(
        "value.quality_only".into(),
        "Every stock with a Piotroski F-score of 7 or more, regardless of price. Monthly.".into(),
        Arc::new(|s: &[Option<Snapshot>]| {
            let pick: Vec<usize> = (0..s.len())
                .filter(|&i| {
                    s[i].as_ref()
                        .is_some_and(|x| x.fscore.is_some_and(|f| f >= 7))
                })
                .collect();
            equal(s.len(), &pick)
        }),
    );
    add(
        "value.universe_equal".into(),
        "Benchmark: every stock with a value score, equal weight, monthly. Value strategies must beat this.".into(),
        Arc::new(|s: &[Option<Snapshot>]| equal(s.len(), &ranked(s, None, |_| true))),
    );
    v
}

// ---------- measurement over time ----------

fn spearman(xs: &[f64], ys: &[f64]) -> Option<f64> {
    if xs.len() < 20 {
        return None;
    }
    let rx = pct_ranks(&xs.iter().map(|x| Some(*x)).collect::<Vec<_>>());
    let ry = pct_ranks(&ys.iter().map(|x| Some(*x)).collect::<Vec<_>>());
    let (a, b): (Vec<f64>, Vec<f64>) = rx
        .into_iter()
        .zip(ry)
        .map(|(a, b)| (a.unwrap(), b.unwrap()))
        .unzip();
    let n = a.len() as f64;
    let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let cov: f64 = a.iter().zip(&b).map(|(x, y)| (x - ma) * (y - mb)).sum();
    let va: f64 = a.iter().map(|x| (x - ma).powi(2)).sum();
    let vb: f64 = b.iter().map(|y| (y - mb).powi(2)).sum();
    (va > 0.0 && vb > 0.0).then(|| cov / (va * vb).sqrt())
}

/// Per signal and period: monthly 1-month ICs, 12-month ICs, and quintile spreads.
type SignalAcc = (Vec<f64>, Vec<f64>, Vec<f64>);

/// All the measurement tables, as JSON rows.
pub struct Measurement {
    pub monthly: Vec<Value>,
    pub signals: Vec<Value>,
}

pub fn measure(panel: &Panel, table: &ScoreTable, validation_start: NaiveDate) -> Measurement {
    let idx: BTreeMap<NaiveDate, usize> = panel
        .dates
        .iter()
        .enumerate()
        .map(|(t, d)| (*d, t))
        .collect();
    let dates = &table.rebalance;
    let close = |t: usize, i: usize| panel.close_at_or_before(t, i);
    let names: Vec<&str> = SIGNALS
        .iter()
        .map(|s| s.0)
        .chain(["value", "fscore", "momentum"])
        .collect();
    let mut monthly = Vec::new();
    // per signal: (period, ic1, ic12, spread)
    let mut acc: BTreeMap<(&str, &str), SignalAcc> = BTreeMap::new();
    for (j, d) in dates.iter().enumerate() {
        let Some(next) = dates.get(j + 1) else { break };
        let (t0, t1) = (idx[d], idx[next]);
        let t12 = dates.get(j + 12).map(|x| idx[x]);
        let snaps = &table.by_date[d];
        let period = if *d < validation_start {
            "Train"
        } else {
            "Validation"
        };
        let fwd = |i: usize, t: usize| Some(close(t, i)? / close(t0, i)? - 1.0);
        let mut row = json!({"date": d, "period": period});
        let mut n_scored = 0;
        for name in &names {
            let score = |s: &Snapshot| -> Option<f64> {
                match *name {
                    "value" => s.value,
                    "fscore" => s.fscore.map(f64::from),
                    "momentum" => s.momentum,
                    _ => s.ranks[SIGNALS.iter().position(|x| x.0 == *name).unwrap()],
                }
            };
            let pairs: Vec<(f64, f64)> = (0..snaps.len())
                .filter_map(|i| Some((score(snaps[i].as_ref()?)?, fwd(i, t1)?)))
                .collect();
            let (xs, ys): (Vec<f64>, Vec<f64>) = pairs.iter().copied().unzip();
            let ic1 = spearman(&xs, &ys);
            let ic12 = t12.and_then(|t| {
                let p: Vec<(f64, f64)> = (0..snaps.len())
                    .filter_map(|i| Some((score(snaps[i].as_ref()?)?, fwd(i, t)?)))
                    .collect();
                let (a, b): (Vec<f64>, Vec<f64>) = p.into_iter().unzip();
                spearman(&a, &b)
            });
            // Top-quintile minus bottom-quintile next-month return.
            let mut sorted = pairs.clone();
            sorted.sort_by(|a, b| b.0.total_cmp(&a.0));
            let k = sorted.len() / 5;
            let spread = (k > 0).then(|| {
                let m = |s: &[(f64, f64)]| s.iter().map(|x| x.1).sum::<f64>() / s.len() as f64;
                m(&sorted[..k]) - m(&sorted[sorted.len() - k..])
            });
            if *name == "value" {
                n_scored = pairs.len();
                for q in 0..5 {
                    let end = if q == 4 { sorted.len() } else { (q + 1) * k };
                    if k > 0 {
                        let s = &sorted[q * k..end];
                        row[format!("q{}", q + 1)] = json!(
                            (s.iter().map(|x| x.1).sum::<f64>() / s.len() as f64 * 1e5).round()
                                / 1e5
                        );
                    }
                }
            }
            let r4 = |x: f64| (x * 1e4).round() / 1e4;
            if let Some(v) = ic1 {
                row[format!("ic_{name}")] = json!(r4(v));
            }
            if let Some(v) = ic12 {
                row[format!("ic12_{name}")] = json!(r4(v));
            }
            let e = acc.entry((name, period)).or_default();
            if let Some(v) = ic1 {
                e.0.push(v);
            }
            if let Some(v) = ic12 {
                e.1.push(v);
            }
            if let Some(v) = spread {
                e.2.push(v);
            }
        }
        row["n"] = json!(n_scored);
        monthly.push(row);
    }
    let label = |n: &str| match n {
        "value" => "Composite value".to_string(),
        "fscore" => "Piotroski F-score".to_string(),
        "momentum" => "12-1 momentum".to_string(),
        _ => SIGNALS.iter().find(|x| x.0 == n).unwrap().1.to_string(),
    };
    let signals = acc
        .into_iter()
        .map(|((name, period), (ic1, ic12, spread))| {
            let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
            let sd = |v: &[f64]| {
                let m = mean(v);
                (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len().max(2) - 1) as f64).sqrt()
            };
            let r4 = |x: f64| if x.is_finite() { (x * 1e4).round() / 1e4 } else { 0.0 };
            json!({
                "signal": name, "label": label(name), "period": period, "months": ic1.len(),
                "mean_ic": r4(mean(&ic1)),
                "ic_t_stat": r4(if sd(&ic1) > 0.0 { mean(&ic1) / sd(&ic1) * (ic1.len() as f64).sqrt() } else { 0.0 }),
                "ic_hit_rate": r4(ic1.iter().filter(|x| **x > 0.0).count() as f64 / ic1.len().max(1) as f64),
                "mean_ic_12m": r4(mean(&ic12)),
                "spread_per_year": r4((1.0 + mean(&spread)).powf(12.0) - 1.0),
            })
        })
        .collect();
    Measurement { monthly, signals }
}

/// Today's ranking: every scored stock with its measures.
pub fn screen(
    panel: &Panel,
    table: &ScoreTable,
    facts: &[Option<Facts>],
) -> (NaiveDate, Vec<Value>) {
    let (date, snaps) = table.by_date.iter().next_back().expect("table has dates");
    let r4 = |x: Option<f64>| x.filter(|v| v.is_finite()).map(|v| (v * 1e4).round() / 1e4);
    let mut rows: Vec<(f64, Value)> = snaps
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let s = s.as_ref()?;
            let v = s.value?;
            let f = facts[i].as_ref()?;
            let flag = match s.fscore {
                Some(x) if x <= 3 => "Weak financials: possible value trap",
                Some(x) if x >= 7 => "Strong financials",
                Some(_) => "",
                None => "F-score unavailable",
            };
            Some((
                v,
                json!({
                    "ticker": panel.symbols[i], "name": f.name, "price": r4(Some(s.price)),
                    "market_cap_bn": r4(Some(s.market_cap / 1e9)), "value_pct": r4(Some(v)),
                    "earnings_yield": r4(s.yields[0]), "book_to_market": r4(s.yields[1]), "sales_yield": r4(s.yields[2]),
                    "fcf_yield": r4(s.yields[3]), "ebit_ev": r4(s.yields[4]),
                    "fscore": s.fscore, "momentum_12_1": r4(s.momentum),
                    "report_filed": s.report_filed, "flag": flag,
                }),
            ))
        })
        .collect();
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    (
        *date,
        rows.into_iter()
            .enumerate()
            .map(|(k, (_, mut v))| {
                v["rank"] = json!(k + 1);
                v
            })
            .collect(),
    )
}

/// Backtest the value strategies plus the benchmarks and random baselines.
pub fn backtest(
    panel: &Panel,
    table: Arc<ScoreTable>,
    base: &[Spec],
    eval_start: NaiveDate,
    validation_start: NaiveDate,
    cash: f64,
    costs: &Costs,
) -> (Vec<Spec>, Vec<BacktestResult>) {
    let mut all = specs(table);
    all.extend(
        base.iter()
            .filter(|s| {
                s.id == "baseline.buy_hold"
                    || s.id.starts_with(crate::strategies::baseline::RANDOM_PREFIX)
            })
            .cloned(),
    );
    let results = backtest::run_all(&all, panel, eval_start, validation_start, cash, costs);
    (all, results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_and_splits() {
        let r = pct_ranks(&[Some(3.0), None, Some(1.0), Some(3.0)]);
        assert_eq!(
            r,
            vec![Some(5.0 / 6.0), None, Some(1.0 / 3.0), Some(5.0 / 6.0)]
        );
        let d = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let splits = [(d("2020-08-31"), 4.0), (d("2014-06-09"), 7.0)];
        assert_eq!(split_adjust(100.0, d("2019-01-01"), &splits), 400.0);
        assert_eq!(split_adjust(100.0, d("2010-01-01"), &splits), 2800.0);
        assert!(
            spearman(
                &(0..30).map(f64::from).collect::<Vec<_>>(),
                &(0..30).map(|x| f64::from(x * 2)).collect::<Vec<_>>()
            )
            .unwrap()
                > 0.999
        );
    }
}
