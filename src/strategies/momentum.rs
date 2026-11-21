//! Time-series momentum: has this asset gone up over the lookback?

use crate::data::Bar;
use crate::indicators::{RetVol, Window};
use crate::strategy::{Side, Spec, SymbolStrategy};
use chrono::NaiveDate;

struct Tsmom {
    closes: Window,
    skip: usize,
    vol: Option<(RetVol, f64)>,
    side: Side,
}
impl SymbolStrategy for Tsmom {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        self.closes.push(b.close);
        let scale = match &mut self.vol {
            Some((v, target)) => match v.update(b.close) {
                Some(sd) if sd > 0.0 => (*target / (sd * 252f64.sqrt())).min(1.0),
                _ => return 0.0,
            },
            None => 1.0,
        };
        if !self.closes.full() {
            return 0.0;
        }
        let r = self.closes.ago(self.skip) / self.closes.first() - 1.0;
        self.side.apply(r.signum()) * scale
    }
}

/// Exposure proportional to how many lookbacks agree (0..1 or -1..1).
struct MultiHorizon {
    closes: Window,
    lags: Vec<usize>,
    side: Side,
}
impl SymbolStrategy for MultiHorizon {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        self.closes.push(b.close);
        if !self.closes.full() {
            return 0.0;
        }
        let s: f64 = self
            .lags
            .iter()
            .map(|&l| (b.close / self.closes.ago(l) - 1.0).signum())
            .sum();
        self.side.apply(s / self.lags.len() as f64)
    }
}

/// 52-week-high proximity (George & Hwang): long when close is near its yearly high.
struct NearHigh {
    highs: Window,
    thresh: f64,
}
impl SymbolStrategy for NearHigh {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        self.highs.push(b.high);
        if self.highs.full() && b.close >= self.thresh * self.highs.max() {
            1.0
        } else {
            0.0
        }
    }
}

pub fn specs() -> Vec<Spec> {
    let mut v = Vec::new();
    for side in Side::BOTH {
        let (st, sw) = (side.tag(), side.words());
        for n in [21, 63, 126, 252] {
            v.push(Spec::per_symbol(
                format!("momentum.tsmom.{n}.{st}"),
                "momentum",
                format!("{sw}: sign of the trailing {n}-day return."),
                move || Tsmom {
                    closes: Window::new(n + 1),
                    skip: 0,
                    vol: None,
                    side,
                },
            ));
            v.push(Spec::per_symbol(
                format!("momentum.tsmom_volscaled.{n}.{st}"),
                "momentum",
                format!("{sw}: {n}-day momentum sign, sized to 15% annualised volatility."),
                move || Tsmom {
                    closes: Window::new(n + 1),
                    skip: 0,
                    vol: Some((RetVol::new(60), 0.15)),
                    side,
                },
            ));
        }
        v.push(Spec::per_symbol(
            format!("momentum.tsmom_12_1.{st}"),
            "momentum",
            format!("{sw}: 12-month return skipping the most recent month."),
            move || Tsmom {
                closes: Window::new(253),
                skip: 21,
                vol: None,
                side,
            },
        ));
        v.push(Spec::per_symbol(
            format!("momentum.multi_horizon.{st}"),
            "momentum",
            format!("{sw}: average sign of 1/3/6/12-month returns."),
            move || MultiHorizon {
                closes: Window::new(253),
                lags: vec![21, 63, 126, 252],
                side,
            },
        ));
    }
    for t in [0.9, 0.95, 0.98] {
        v.push(Spec::per_symbol(
            format!("momentum.near_52w_high.{t}"),
            "momentum",
            format!(
                "Long when close is within {:.0}% of its 52-week high.",
                (1.0 - t) * 100.0
            ),
            move || NearHigh {
                highs: Window::new(252),
                thresh: t,
            },
        ));
    }
    v
}
