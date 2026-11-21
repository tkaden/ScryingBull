//! Market data: bars, on-disk CSV cache, and the aligned multi-symbol panel.

/// Test fixture only: never used for real backtests or the site.
#[cfg(test)]
pub mod synthetic;
pub mod yahoo;

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

/// One daily OHLCV bar. Prices are fully adjusted (splits + dividends).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Bar {
    pub fn is_sane(&self) -> bool {
        [self.open, self.high, self.low, self.close]
            .iter()
            .all(|p| p.is_finite() && *p > 0.0)
            && self.volume.is_finite()
            && self.volume >= 0.0
    }
}

#[derive(Clone, Debug)]
pub struct Series {
    pub symbol: String,
    pub bars: Vec<Bar>,
}

impl Series {
    pub fn to_csv(&self) -> String {
        let mut s = String::from("date,open,high,low,close,volume\n");
        for b in &self.bars {
            let _ = writeln!(
                s,
                "{},{},{},{},{},{}",
                b.date, b.open, b.high, b.low, b.close, b.volume
            );
        }
        s
    }

    pub fn from_csv(symbol: &str, text: &str) -> Result<Series> {
        let mut bars = Vec::new();
        for (i, line) in text.lines().enumerate().skip(1) {
            if line.trim().is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split(',').collect();
            if f.len() < 6 {
                bail!("{symbol}: line {} has {} fields", i + 1, f.len());
            }
            let num = |k: usize| -> Result<f64> {
                f[k].trim()
                    .parse::<f64>()
                    .with_context(|| format!("{symbol}: line {} field {k}", i + 1))
            };
            let bar = Bar {
                date: NaiveDate::parse_from_str(f[0].trim(), "%Y-%m-%d")
                    .with_context(|| format!("{symbol}: bad date on line {}", i + 1))?,
                open: num(1)?,
                high: num(2)?,
                low: num(3)?,
                close: num(4)?,
                volume: num(5)?,
            };
            if bar.is_sane() {
                bars.push(bar);
            }
        }
        bars.sort_by_key(|b| b.date);
        bars.dedup_by_key(|b| b.date);
        Ok(Series {
            symbol: symbol.to_string(),
            bars,
        })
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(format!("{}.csv", self.symbol)), self.to_csv())?;
        Ok(())
    }

    pub fn load(dir: &Path, symbol: &str) -> Result<Series> {
        let path = dir.join(format!("{symbol}.csv"));
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "missing data for {symbol} at {} (run `scry fetch`)",
                path.display()
            )
        })?;
        Series::from_csv(symbol, &text)
    }
}

/// All symbols aligned on the union of their trading dates.
/// `bars[t][i]` is `Some` only if symbol `i` actually traded on `dates[t]`.
pub struct Panel {
    pub symbols: Vec<String>,
    pub dates: Vec<NaiveDate>,
    pub bars: Vec<Vec<Option<Bar>>>,
}

impl Panel {
    pub fn new(series: Vec<Series>, start: NaiveDate) -> Panel {
        let dates: Vec<NaiveDate> = series
            .iter()
            .flat_map(|s| s.bars.iter().map(|b| b.date))
            .filter(|d| *d >= start)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut bars = vec![vec![None; series.len()]; dates.len()];
        for (i, s) in series.iter().enumerate() {
            for b in &s.bars {
                if let Ok(t) = dates.binary_search(&b.date) {
                    bars[t][i] = Some(*b);
                }
            }
        }
        Panel {
            symbols: series.into_iter().map(|s| s.symbol).collect(),
            dates,
            bars,
        }
    }

    pub fn load(dir: &Path, symbols: &[String], start: NaiveDate) -> Result<Panel> {
        let mut series = Vec::new();
        for s in symbols {
            match Series::load(dir, s) {
                Ok(x) if !x.bars.is_empty() => series.push(x),
                Ok(_) => eprintln!("warning: {s} has no bars, skipping"),
                Err(e) => eprintln!("warning: {e:#}, skipping"),
            }
        }
        if series.is_empty() {
            bail!(
                "no market data found in {} (run `scry fetch` or `scry synth`)",
                dir.display()
            );
        }
        Ok(Panel::new(series, start))
    }

    pub fn len(&self) -> usize {
        self.dates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dates.is_empty()
    }

    /// Most recent close for symbol `i` at or before `t`.
    pub fn close_at_or_before(&self, t: usize, i: usize) -> Option<f64> {
        (0..=t).rev().find_map(|k| self.bars[k][i].map(|b| b.close))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_roundtrip() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
        let s = Series {
            symbol: "X".into(),
            bars: vec![Bar {
                date: d,
                open: 1.0,
                high: 2.0,
                low: 0.5,
                close: 1.5,
                volume: 10.0,
            }],
        };
        let back = Series::from_csv("X", &s.to_csv()).unwrap();
        assert_eq!(back.bars, s.bars);
    }

    #[test]
    fn panel_aligns_on_union_of_dates() {
        let d = |k| NaiveDate::from_ymd_opt(2024, 1, k).unwrap();
        let bar = |k| Bar {
            date: d(k),
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 0.0,
        };
        let a = Series {
            symbol: "A".into(),
            bars: vec![bar(2), bar(3)],
        };
        let b = Series {
            symbol: "B".into(),
            bars: vec![bar(3), bar(4)],
        };
        let p = Panel::new(vec![a, b], d(1));
        assert_eq!(p.dates, vec![d(2), d(3), d(4)]);
        assert!(p.bars[0][1].is_none() && p.bars[2][0].is_none() && p.bars[1][0].is_some());
    }
}
