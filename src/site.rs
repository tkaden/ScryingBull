//! Static website: a leaderboard of every strategy, backtest vs. paper.
//! Output is plain files (HTML/JS/CSS + JSON) suitable for GitHub Pages.

use crate::backtest::{validation_report, BacktestResult};
use crate::config::Config;
use crate::metrics::compute;
use crate::paper::Ledger;
use crate::strategies::baseline::RANDOM_PREFIX;
use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use std::path::Path;

const INDEX: &str = include_str!("../web/index.html");
const APP: &str = include_str!("../web/app.js");
const STYLE: &str = include_str!("../web/style.css");
const VALUE_HTML: &str = include_str!("../web/value.html");
const VALUE_JS: &str = include_str!("../web/value.js");

/// Marker written next to the data cache recording where the data came from.
pub const SOURCE_FILE: &str = "SOURCE";

fn r4(x: f64) -> f64 {
    if x.is_finite() {
        (x * 1e4).round() / 1e4
    } else {
        0.0
    }
}

pub fn build(out: &Path, cfg: &Config, results: &[BacktestResult], ledger: &Ledger) -> Result<()> {
    let curves = out.join("data/curves");
    std::fs::create_dir_all(&curves)?;
    std::fs::write(out.join("index.html"), INDEX)?;
    std::fs::write(out.join("app.js"), APP)?;
    std::fs::write(out.join("style.css"), STYLE)?;
    std::fs::write(out.join(".nojekyll"), "")?;

    let source = std::fs::read_to_string(cfg.data_dir.join(SOURCE_FILE))
        .unwrap_or_else(|_| "unknown".into());
    let mut rows = Vec::new();
    let mut ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    for id in ledger.accounts.keys() {
        if !ids.contains(&id.as_str()) {
            ids.push(id);
        }
    }
    for id in ids {
        let bt = results.iter().find(|r| r.id == id);
        let acc = ledger.accounts.get(id);
        let paper = acc.map(|a| {
            let m = compute(&a.equity);
            json!({
                "inception": a.inception, "days": a.equity.len().saturating_sub(1), "active": a.active,
                "ret": r4(m.total_return), "sharpe": r4(m.sharpe), "mdd": r4(m.max_drawdown),
                "trades": a.trades, "equity": r4(a.equity.last().map_or(0.0, |x| x.1)),
            })
        });
        let row = json!({
            "id": id,
            "family": bt.map(|b| b.family.clone()).or(acc.map(|a| a.family.clone())),
            "description": bt.map(|b| b.description.clone()).unwrap_or_default(),
            "random": id.starts_with(RANDOM_PREFIX),
            "bt": bt.map(|b| { let m = &b.metrics; json!({
                "cagr": r4(m.cagr), "sharpe": r4(m.sharpe), "sortino": r4(m.sortino), "mdd": r4(m.max_drawdown),
                "vol": r4(m.vol), "total": r4(m.total_return), "calmar": r4(m.calmar), "hit": r4(m.hit_rate),
                "ret1y": r4(b.last_year.total_return), "sharpe1y": r4(b.last_year.sharpe),
                "trades": b.trades, "turnover": r4(b.turnover), "exposure": r4(b.avg_exposure),
                "beats_random_train": r4(b.beats_random_train),
                "beats_random_val": r4(b.beats_random_validation),
                "train": { "cagr": r4(b.train.cagr), "sharpe": r4(b.train.sharpe), "mdd": r4(b.train.max_drawdown) },
                "val": { "cagr": r4(b.validation.cagr), "sharpe": r4(b.validation.sharpe), "mdd": r4(b.validation.max_drawdown), "total": r4(b.validation.total_return) },
                "yearly": m.yearly.iter().map(|(y, r)| (y.to_string(), Value::from(r4(*r)))).collect::<serde_json::Map<_, _>>(),
            })}),
            "paper": paper,
        });
        rows.push(row);
        let curve = json!({
            "bt": bt.map(|b| b.curve.iter().map(|(d, v)| json!([d, v])).collect::<Vec<_>>()).unwrap_or_default(),
            "paper": acc.map(|a| a.equity.iter().map(|(d, v)| json!([d, v])).collect::<Vec<_>>()).unwrap_or_default(),
        });
        std::fs::write(
            curves.join(format!("{id}.json")),
            serde_json::to_string(&curve)?,
        )?;
    }
    let summary = json!({
        "generated": Utc::now().to_rfc3339(),
        "source": source.trim(),
        "eval_start": cfg.eval_start,
        "validation_start": cfg.validation_start,
        "validation": validation_report(results),
        "data_end": results.first().and_then(|r| r.metrics.end),
        "paper_date": ledger.last_date,
        "universe": cfg.universe,
        "costs": cfg.costs,
        "strategies": rows,
    });
    std::fs::write(
        out.join("data/summary.json"),
        serde_json::to_string(&summary)?,
    )?;
    Ok(())
}

/// Add the value-research page when `scry value run` has produced results.
pub fn build_value(out: &Path, results: &Path, ledger: &Ledger) -> Result<bool> {
    let Ok(screen) = std::fs::read_to_string(results.join("screen.json")) else {
        return Ok(false);
    };
    let dir = out.join("data/value");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(out.join("value.html"), VALUE_HTML)?;
    std::fs::write(out.join("value.js"), VALUE_JS)?;
    std::fs::write(dir.join("screen.json"), screen)?;
    for f in ["signals.json", "monthly.json"] {
        std::fs::copy(results.join(f), dir.join(f))?;
    }
    let bt: Vec<BacktestResult> =
        serde_json::from_str(&std::fs::read_to_string(results.join("backtest.json"))?)?;
    let rows: Vec<Value> = bt
        .iter()
        .filter(|b| !b.id.starts_with(RANDOM_PREFIX))
        .map(|b| {
            let paper = ledger.accounts.get(&b.id).map(|a| {
                let m = compute(&a.equity);
                json!({"days": a.equity.len().saturating_sub(1), "ret": r4(m.total_return), "sharpe": r4(m.sharpe), "inception": a.inception})
            });
            json!({
                "id": b.id, "description": b.description,
                "train": {"cagr": r4(b.train.cagr), "sharpe": r4(b.train.sharpe), "mdd": r4(b.train.max_drawdown)},
                "val": {"cagr": r4(b.validation.cagr), "sharpe": r4(b.validation.sharpe), "mdd": r4(b.validation.max_drawdown)},
                "beats_random_val": r4(b.beats_random_validation), "turnover": r4(b.turnover),
                "paper": paper,
            })
        })
        .collect();
    let random_sharpes: Vec<f64> = bt
        .iter()
        .filter(|b| b.id.starts_with(RANDOM_PREFIX))
        .map(|b| b.validation.sharpe)
        .collect();
    std::fs::write(
        dir.join("strategies.json"),
        serde_json::to_string(
            &json!({"generated": Utc::now().to_rfc3339(), "strategies": rows, "random_val_sharpe": random_sharpes}),
        )?,
    )?;
    Ok(true)
}
