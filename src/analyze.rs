//! Deep-dive analytics for one strategy: `scry analyze <id>`.
//!
//! Writes one JSON file per table, each `{rows, meta}`, for dashboards or notebooks.
//! Everything is computed from the same engine and data as the backtests.

use crate::backtest::split;
use crate::config::Config;
use crate::data::Panel;
use crate::engine::{simulate, Costs, Run};
use crate::metrics::{compute, Metrics};
use crate::strategies::baseline::RANDOM_PREFIX;
use crate::strategy::Spec;
use anyhow::{bail, Result};
use chrono::{Datelike, NaiveDate};
use rayon::prelude::*;
use serde_json::{json, Value};
use std::path::Path;

const BENCH: &str = "baseline.buy_hold";

fn r(x: f64, dp: i32) -> f64 {
    let k = 10f64.powi(dp);
    if x.is_finite() {
        (x * k).round() / k
    } else {
        0.0
    }
}

fn find<'a>(specs: &'a [Spec], id: &str) -> Result<&'a Spec> {
    specs
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| anyhow::anyhow!("unknown strategy {id} (see `scry list`)"))
}

fn run(spec: &Spec, panel: &Panel, cfg: &Config, costs: &Costs) -> Run {
    simulate(
        (spec.build)().as_mut(),
        panel,
        cfg.eval_start,
        cfg.cash,
        costs,
    )
}

fn metrics_row(series: &str, period: &str, m: &Metrics) -> Value {
    json!({
        "series": series, "period": period,
        "start": m.start, "end": m.end, "days": m.days,
        "total_return": r(m.total_return, 4), "cagr": r(m.cagr, 4), "vol": r(m.vol, 4),
        "sharpe": r(m.sharpe, 3), "sortino": r(m.sortino, 3), "max_drawdown": r(m.max_drawdown, 4),
        "calmar": r(m.calmar, 3), "hit_rate": r(m.hit_rate, 4),
    })
}

fn drawdown_series(eq: &[(NaiveDate, f64)]) -> Vec<f64> {
    let mut peak = f64::MIN;
    eq.iter()
        .map(|(_, v)| {
            peak = peak.max(*v);
            v / peak - 1.0
        })
        .collect()
}

/// The `k` deepest non-overlapping drawdown episodes: (peak, trough, recovery, depth).
fn episodes(eq: &[(NaiveDate, f64)], k: usize) -> Vec<(usize, usize, Option<usize>, f64)> {
    let mut out = Vec::new();
    let (mut peak_i, mut trough_i) = (0, 0);
    let mut in_dd = false;
    for i in 1..eq.len() {
        if eq[i].1 >= eq[peak_i].1 {
            if in_dd {
                out.push((
                    peak_i,
                    trough_i,
                    Some(i),
                    eq[trough_i].1 / eq[peak_i].1 - 1.0,
                ));
                in_dd = false;
            }
            peak_i = i;
            trough_i = i;
        } else {
            in_dd = true;
            if eq[i].1 < eq[trough_i].1 {
                trough_i = i;
            }
        }
    }
    if in_dd {
        out.push((peak_i, trough_i, None, eq[trough_i].1 / eq[peak_i].1 - 1.0));
    }
    out.sort_by(|a, b| a.3.total_cmp(&b.3));
    out.truncate(k);
    out
}

fn rolling_sharpe(eq: &[(NaiveDate, f64)], window: usize) -> Vec<Option<f64>> {
    let rets: Vec<f64> = std::iter::once(0.0)
        .chain(eq.windows(2).map(|w| w[1].1 / w[0].1 - 1.0))
        .collect();
    (0..eq.len())
        .map(|i| {
            if i < window {
                return None;
            }
            let s = &rets[i + 1 - window..=i];
            let m = s.iter().sum::<f64>() / window as f64;
            let sd = (s.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (window - 1) as f64).sqrt();
            (sd > 0.0).then(|| m / sd * 252f64.sqrt())
        })
        .collect()
}

fn month_ends(eq: &[(NaiveDate, f64)]) -> Vec<(NaiveDate, f64)> {
    let mut out: Vec<(NaiveDate, f64)> = Vec::new();
    for (k, &(d, v)) in eq.iter().enumerate() {
        if eq.get(k + 1).is_none_or(|(n, _)| n.month() != d.month()) {
            out.push((d, v));
        }
    }
    out
}

pub fn analyze(
    cfg: &Config,
    panel: &Panel,
    alt: Option<&Panel>,
    specs: &[Spec],
    id: &str,
    out: &Path,
) -> Result<()> {
    let spec = find(specs, id)?;
    let bench = find(specs, BENCH)?;
    if id == BENCH {
        bail!("pick a strategy other than the benchmark");
    }
    std::fs::create_dir_all(out)?;
    let costs = cfg.costs;
    let (s, b) = rayon::join(
        || run(spec, panel, cfg, &costs),
        || run(bench, panel, cfg, &costs),
    );
    assert_eq!(s.equity.len(), b.equity.len());
    let vs = cfg.validation_start;
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let data_end = panel.dates.last().copied();
    let meta = |what: &str| {
        json!([
            {"label": "Command", "value": format!("scry analyze {id}"), "type": "code", "lang": "bash"},
            {"label": "Code", "value": format!("ScryingBull src/analyze.rs @ {commit}")},
            {"label": "Data", "value": format!("Yahoo Finance daily bars, split/dividend adjusted, {} symbols, through {}", panel.symbols.len(), data_end.map(|d| d.to_string()).unwrap_or_default())},
            {"label": "Execution", "value": format!("signals at the close, fills at the next open; {} bps slippage + {} bps commission", costs.slippage_bps, costs.commission_bps)},
            {"label": "Table", "value": what},
        ])
    };
    let write = |name: &str, rows: Vec<Value>, what: &str| -> Result<()> {
        std::fs::write(
            out.join(format!("{name}.json")),
            serde_json::to_string(&json!({"rows": rows, "meta": meta(what)}))?,
        )?;
        Ok(())
    };

    // 1. Daily curves.
    let (sdd, bdd) = (drawdown_series(&s.equity), drawdown_series(&b.equity));
    let (s0, b0) = (s.equity[0].1, b.equity[0].1);
    let daily: Vec<Value> = (0..s.equity.len())
        .map(|i| {
            json!({
                "date": s.equity[i].0, "period": if s.equity[i].0 < vs { "Train" } else { "Validation" },
                "strategy": r(s.equity[i].1 / s0, 5), "buy_hold": r(b.equity[i].1 / b0, 5),
                "strategy_dd": r(sdd[i], 5), "buy_hold_dd": r(bdd[i], 5),
                "exposure": r(s.exposure[i], 4), "buy_hold_exposure": r(b.exposure[i], 4),
            })
        })
        .collect();
    write("daily", daily, "Daily growth of $1, drawdown from peak and gross exposure, strategy vs equal-weight buy & hold")?;

    // 2. Summary by period, with the random-baseline median for context.
    let periods = |eq: &[(NaiveDate, f64)]| {
        let (t, v) = split(eq, vs);
        [
            ("Train", compute(t)),
            ("Validation", compute(v)),
            ("Full", compute(eq)),
        ]
    };
    let mut summary = Vec::new();
    for (name, run) in [("Strategy", &s), ("Buy & hold", &b)] {
        for (p, m) in periods(&run.equity) {
            let mut row = metrics_row(name, p, &m);
            row["avg_exposure"] = json!(r(run.avg_exposure, 4));
            summary.push(row);
        }
    }
    let randoms: Vec<&Spec> = specs
        .iter()
        .filter(|x| x.id.starts_with(RANDOM_PREFIX))
        .collect();
    let rand_runs: Vec<Run> = randoms
        .par_iter()
        .map(|x| run(x, panel, cfg, &costs))
        .collect();
    for (pi, p) in ["Train", "Validation", "Full"].iter().enumerate() {
        let mut sh: Vec<f64> = rand_runs
            .iter()
            .map(|rr| periods(&rr.equity)[pi].1.sharpe)
            .collect();
        sh.sort_by(f64::total_cmp);
        let q = |f: f64| sh[((sh.len() - 1) as f64 * f).round() as usize];
        summary.push(json!({"series": "Random baselines", "period": p, "sharpe": r(q(0.5), 3), "sharpe_p05": r(q(0.05), 3), "sharpe_p95": r(q(0.95), 3)}));
    }
    let years = s.equity.len() as f64 / 252.0;
    summary.push(json!({"series": "Strategy", "period": "Trading", "trades": s.book.trades, "turnover_per_year": r(s.book.turnover / years, 3)}));
    write("summary", summary, "Performance by period (train / validation / full) for the strategy, buy & hold, and 20 random strategies")?;

    // 3. Monthly returns.
    let (sm, bm) = (month_ends(&s.equity), month_ends(&b.equity));
    let monthly: Vec<Value> = (0..sm.len())
        .map(|i| {
            let (sp, bp) = if i == 0 { (s0, b0) } else { (sm[i - 1].1, bm[i - 1].1) };
            json!({"year": sm[i].0.year(), "month": sm[i].0.month(), "strategy": r(sm[i].1 / sp - 1.0, 5), "buy_hold": r(bm[i].1 / bp - 1.0, 5)})
        })
        .collect();
    write("monthly", monthly, "Calendar-month returns")?;

    // 4. Rolling one-year Sharpe, sampled weekly.
    let (srs, brs) = (
        rolling_sharpe(&s.equity, 252),
        rolling_sharpe(&b.equity, 252),
    );
    let rolling: Vec<Value> = (0..s.equity.len())
        .filter(|i| i % 5 == 0 && srs[*i].is_some())
        .map(|i| json!({"date": s.equity[i].0, "strategy": r(srs[i].unwrap(), 3), "buy_hold": r(brs[i].unwrap_or(0.0), 3)}))
        .collect();
    write(
        "rolling",
        rolling,
        "Trailing 252-day Sharpe ratio, sampled weekly",
    )?;

    // 5. Worst drawdowns, and what buy & hold did over the same window.
    let dds: Vec<Value> = episodes(&s.equity, 6)
        .into_iter()
        .enumerate()
        .map(|(k, (p, t, rec, depth))| {
            json!({
                "rank": k + 1, "peak": s.equity[p].0, "trough": s.equity[t].0,
                "recovered": rec.map(|i| s.equity[i].0.to_string()).unwrap_or_else(|| "not yet".into()),
                "depth": r(depth, 4), "days_down": t - p, "days_to_recover": rec.map(|i| i - t),
                "buy_hold_same_window": r(b.equity[t].1 / b.equity[p].1 - 1.0, 4),
            })
        })
        .collect();
    write("drawdowns", dds, "Deepest drawdown episodes (trading days) and buy & hold's return from the same peak to trough")?;

    // 6. Parameter neighbours: every registered variant of the same method.
    let prefix: String = id.split('.').take(2).collect::<Vec<_>>().join(".") + ".";
    let sibs: Vec<&Spec> = specs.iter().filter(|x| x.id.starts_with(&prefix)).collect();
    let sens: Vec<Value> = sibs
        .par_iter()
        .map(|x| {
            let rr = run(x, panel, cfg, &costs);
            let [(_, t), (_, v), _] = periods(&rr.equity);
            let rest = &x.id[prefix.len()..];
            let (p1, p2) = rest.split_once('.').unwrap_or((rest, ""));
            json!({
                "id": x.id, "param1": p1, "param2": p2, "selected": x.id == id,
                "train_sharpe": r(t.sharpe, 3), "val_sharpe": r(v.sharpe, 3), "val_cagr": r(v.cagr, 4),
                "val_max_drawdown": r(v.max_drawdown, 4), "avg_exposure": r(rr.avg_exposure, 4),
            })
        })
        .collect();
    write(
        "sensitivity",
        sens,
        &format!("Every registered variant of {prefix}* (param1, param2 as in the id)"),
    )?;

    // 7. Trading-cost stress test.
    let levels = [
        (0.0, 0.0),
        (costs.commission_bps, costs.slippage_bps),
        (2.0, 10.0),
        (5.0, 20.0),
        (10.0, 40.0),
    ];
    let cost_rows: Vec<Value> = levels
        .par_iter()
        .map(|&(c, sl)| {
            let k = Costs { commission_bps: c, slippage_bps: sl, min_trade: costs.min_trade };
            let rr = run(spec, panel, cfg, &k);
            let bb = run(bench, panel, cfg, &k);
            let [(_, t), (_, v), (_, f)] = periods(&rr.equity);
            let [_, (_, bv), _] = periods(&bb.equity);
            json!({
                "cost_bps": c + sl, "commission_bps": c, "slippage_bps": sl, "is_default": c == costs.commission_bps && sl == costs.slippage_bps,
                "train_sharpe": r(t.sharpe, 3), "val_sharpe": r(v.sharpe, 3), "val_cagr": r(v.cagr, 4), "full_cagr": r(f.cagr, 4),
                "buy_hold_val_sharpe": r(bv.sharpe, 3),
            })
        })
        .collect();
    write(
        "costs",
        cost_rows,
        "Results at increasing trading costs per trade (commission + slippage, basis points)",
    )?;

    // 8. Universe robustness.
    let mut uni = Vec::new();
    for (label, p) in std::iter::once((format!("Default ({} symbols)", panel.symbols.len()), panel))
        .chain(alt.map(|a| (format!("ETFs only ({} symbols)", a.symbols.len()), a)))
    {
        for (series, sp) in [("Strategy", spec), ("Buy & hold", bench)] {
            let rr = run(sp, p, cfg, &costs);
            let [(_, t), (_, v), _] = periods(&rr.equity);
            uni.push(json!({
                "universe": label, "series": series,
                "train_sharpe": r(t.sharpe, 3), "train_cagr": r(t.cagr, 4), "val_sharpe": r(v.sharpe, 3),
                "val_cagr": r(v.cagr, 4), "val_max_drawdown": r(v.max_drawdown, 4),
            }));
        }
    }
    write(
        "universes",
        uni,
        "Same strategy and benchmark re-run on a second universe with no hindsight stock picks",
    )?;
    println!("wrote analytics for {id} to {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_drawdown_episodes() {
        let d = |k| NaiveDate::from_ymd_opt(2024, 1, k).unwrap();
        let eq: Vec<_> = [100.0, 90.0, 80.0, 100.0, 110.0, 99.0, 120.0, 60.0]
            .iter()
            .enumerate()
            .map(|(i, v)| (d(i as u32 + 1), *v))
            .collect();
        let e = episodes(&eq, 3);
        assert_eq!(e[0], (6, 7, None, -0.5));
        assert_eq!(e[1], (0, 2, Some(3), -0.19999999999999996));
        assert_eq!(e.len(), 3);
    }
}
