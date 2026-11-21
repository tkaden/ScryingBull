//! Forward paper trading for every strategy at once.
//!
//! The ledger is append-only bookkeeping: each run fills the orders decided at
//! the previous run's close at the next open, marks to market, then records
//! today's new orders. Strategies are replayed from history to rebuild their
//! internal state (they are deterministic), but only *new* days touch the books,
//! so past paper results can never be rewritten by a code or data change.

use crate::data::Panel;
use crate::engine::{Book, Costs};
use crate::strategy::Spec;
use anyhow::{Context, Result};
use chrono::NaiveDate;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    pub version: u32,
    /// Last market date processed.
    pub last_date: Option<NaiveDate>,
    /// Reference close per symbol at `last_date`, used to detect splits/dividends
    /// (adjusted history is rescaled by the data vendor; positions are rescaled to match).
    pub ref_close: BTreeMap<String, (NaiveDate, f64)>,
    pub accounts: BTreeMap<String, Account>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub family: String,
    /// First close at which this account placed orders.
    pub inception: NaiveDate,
    pub active: bool,
    pub cash: f64,
    pub positions: BTreeMap<String, Position>,
    pub equity: Vec<(NaiveDate, f64)>,
    pub trades: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Position {
    pub qty: f64,
    pub last_px: f64,
    pub target: f64,
    /// Order decided but not yet filled.
    pub pending: bool,
}

impl Ledger {
    pub fn load(path: &Path) -> Result<Ledger> {
        if !path.exists() {
            return Ok(Ledger {
                version: 1,
                ..Default::default()
            });
        }
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).with_context(|| format!("parsing ledger {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}

fn to_book(acc: &Account, symbols: &[String]) -> (Book, f64) {
    let mut book = Book::new(acc.cash, symbols.len());
    let mut orphan_value = 0.0;
    for (sym, p) in &acc.positions {
        match symbols.iter().position(|s| s == sym) {
            Some(i) => {
                book.qty[i] = p.qty;
                book.last_px[i] = p.last_px;
                book.target[i] = p.target;
                book.dirty[i] = p.pending;
            }
            // Symbol left the universe: liquidate at its last known price.
            None => orphan_value += p.qty * p.last_px,
        }
    }
    book.cash += orphan_value;
    book.trades = acc.trades;
    (book, orphan_value)
}

fn from_book(acc: &mut Account, book: &Book, symbols: &[String]) {
    acc.cash = book.cash;
    acc.trades = book.trades;
    acc.positions = symbols
        .iter()
        .enumerate()
        .filter(|(i, _)| book.qty[*i] != 0.0 || book.target[*i] != 0.0 || book.dirty[*i])
        .map(|(i, s)| {
            (
                s.clone(),
                Position {
                    qty: book.qty[i],
                    last_px: book.last_px[i],
                    target: book.target[i],
                    pending: book.dirty[i],
                },
            )
        })
        .collect();
}

#[derive(Debug, Default)]
pub struct Report {
    pub new_days: usize,
    pub opened: usize,
    pub updated: usize,
    pub adjustments: Vec<String>,
}

/// Advance the ledger to the end of `panel`.
pub fn advance(
    ledger: &mut Ledger,
    panel: &Panel,
    specs: &[Spec],
    cash: f64,
    costs: &Costs,
) -> Report {
    let mut report = Report::default();
    let Some(&end) = panel.dates.last() else {
        return report;
    };

    // 1. Corporate actions: rescale positions if the vendor re-adjusted history.
    for (sym, (date, old)) in ledger.ref_close.clone() {
        let (Some(i), Ok(t)) = (
            panel.symbols.iter().position(|s| *s == sym),
            panel.dates.binary_search(&date),
        ) else {
            continue;
        };
        let Some(new) = panel.bars[t][i].map(|b| b.close) else {
            continue;
        };
        let ratio = old / new;
        if (ratio - 1.0).abs() > 1e-6 {
            report.adjustments.push(format!(
                "{sym}: history rescaled by {:.6}, positions adjusted",
                1.0 / ratio
            ));
            for acc in ledger.accounts.values_mut() {
                if let Some(p) = acc.positions.get_mut(&sym) {
                    p.qty *= ratio;
                    p.last_px /= ratio;
                }
            }
        }
    }

    let last = ledger.last_date;
    report.new_days = last.map_or(0, |l| panel.dates.iter().filter(|d| **d > l).count());

    // 2. Replay every strategy; touch the books only for new days.
    let n = panel.symbols.len();
    let updates: Vec<(String, Account, bool)> = specs
        .par_iter()
        .map(|spec| {
            let existing = ledger.accounts.get(&spec.id).filter(|_| last.is_some());
            let mut strat = (spec.build)();
            let mut w = vec![f64::NAN; n];
            match existing {
                Some(acc) => {
                    let mut acc = acc.clone();
                    let (mut book, _) = to_book(&acc, &panel.symbols);
                    let last = last.unwrap();
                    for (t, date) in panel.dates.iter().enumerate() {
                        let bars = &panel.bars[t];
                        let live = *date > last;
                        if live {
                            book.execute(bars, costs);
                            let eq = book.mark(bars);
                            acc.equity.push((*date, (eq * 100.0).round() / 100.0));
                        }
                        w.fill(f64::NAN);
                        strat.next(*date, bars, &mut w);
                        if live {
                            book.set_targets(&w);
                        }
                    }
                    from_book(&mut acc, &book, &panel.symbols);
                    acc.active = true;
                    (spec.id.clone(), acc, false)
                }
                None => {
                    // New account: rebuild sticky targets from history, start trading tomorrow.
                    let mut book = Book::new(cash, n);
                    for (t, date) in panel.dates.iter().enumerate() {
                        book.mark(&panel.bars[t]);
                        w.fill(f64::NAN);
                        strat.next(*date, &panel.bars[t], &mut w);
                        book.set_targets(&w);
                    }
                    book.arm_all();
                    let mut acc = Account {
                        family: spec.family.to_string(),
                        inception: end,
                        active: true,
                        cash,
                        positions: BTreeMap::new(),
                        equity: vec![(end, cash)],
                        trades: 0,
                    };
                    from_book(&mut acc, &book, &panel.symbols);
                    (spec.id.clone(), acc, true)
                }
            }
        })
        .collect();

    for acc in ledger.accounts.values_mut() {
        acc.active = false;
    }
    for (id, acc, new) in updates {
        if new {
            report.opened += 1;
        } else {
            report.updated += 1;
        }
        ledger.accounts.insert(id, acc);
    }

    // 3. New reference closes.
    for (i, sym) in panel.symbols.iter().enumerate() {
        if let Some(t) = (0..panel.len()).rev().find(|t| panel.bars[*t][i].is_some()) {
            ledger.ref_close.insert(
                sym.clone(),
                (panel.dates[t], panel.bars[t][i].unwrap().close),
            );
        }
    }
    ledger.last_date = Some(end);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{synthetic, Bar};
    use crate::engine::simulate;
    use crate::strategies::registry;

    fn panel_until(end: NaiveDate) -> Panel {
        let syms: Vec<String> = ["AAA", "BBB", "CCC", "DDD"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let d0 = NaiveDate::from_ymd_opt(2018, 1, 1).unwrap();
        let mut series =
            synthetic::generate(&syms, d0, NaiveDate::from_ymd_opt(2021, 1, 1).unwrap(), 3);
        for s in &mut series {
            s.bars.retain(|b: &Bar| b.date <= end);
        }
        Panel::new(series, d0)
    }

    /// Paper trading day-by-day must match a backtest started at the same date.
    #[test]
    fn paper_matches_backtest() {
        let specs: Vec<Spec> = registry()
            .into_iter()
            .filter(|s| {
                [
                    "baseline.buy_hold",
                    "trend.ma_cross.ema.10x50.ls",
                    "cross_sectional.momentum.63.0.top3",
                ]
                .contains(&s.id.as_str())
            })
            .collect();
        assert_eq!(specs.len(), 3);
        let costs = Costs::default();
        let full = panel_until(NaiveDate::from_ymd_opt(2021, 1, 1).unwrap());
        let start_idx = full.dates.len() - 120;
        let mut ledger = Ledger::default();
        for k in start_idx..full.dates.len() {
            let p = panel_until(full.dates[k]);
            advance(&mut ledger, &p, &specs, 1e5, &costs);
        }
        for spec in &specs {
            let acc = &ledger.accounts[&spec.id];
            // Backtest that starts trading the day after inception.
            let run = simulate(
                (spec.build)().as_mut(),
                &full,
                full.dates[start_idx + 1],
                1e5,
                &costs,
            );
            let (pe, be) = (acc.equity.last().unwrap().1, run.equity.last().unwrap().1);
            assert!(
                (pe - be).abs() < 0.02,
                "{}: paper {pe} vs backtest {be}",
                spec.id
            );
        }
    }

    #[test]
    fn rescales_positions_on_split() {
        let specs: Vec<Spec> = registry()
            .into_iter()
            .filter(|s| s.id == "baseline.buy_hold")
            .collect();
        let costs = Costs::default();
        let mut ledger = Ledger::default();
        advance(
            &mut ledger,
            &panel_until(NaiveDate::from_ymd_opt(2020, 6, 1).unwrap()),
            &specs,
            1e5,
            &costs,
        );
        let mut p = panel_until(NaiveDate::from_ymd_opt(2020, 6, 2).unwrap());
        advance(&mut ledger, &p, &specs, 1e5, &costs); // fills at the open
        let pos = ledger.accounts["baseline.buy_hold"].positions["AAA"].clone();
        assert!(pos.qty > 0.0);
        // The vendor re-adjusts all of AAA's history for a 2:1 split.
        let i = p.symbols.iter().position(|s| s == "AAA").unwrap();
        for b in p.bars.iter_mut().filter_map(|r| r[i].as_mut()) {
            b.open *= 0.5;
            b.high *= 0.5;
            b.low *= 0.5;
            b.close *= 0.5;
        }
        let rep = advance(&mut ledger, &p, &specs, 1e5, &costs);
        assert_eq!(rep.adjustments.len(), 1);
        let after = &ledger.accounts["baseline.buy_hold"].positions["AAA"];
        assert!((after.qty - 2.0 * pos.qty).abs() < 1e-9);
        assert!((after.qty * after.last_px - pos.qty * pos.last_px).abs() < 1e-6);
    }
}
