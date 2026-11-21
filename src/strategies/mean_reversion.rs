//! Mean reversion: buy short-term weakness, sell short-term strength.

use crate::data::Bar;
use crate::indicators::{Rsi, Sma, Window};
use crate::strategy::{Side, Spec, SymbolStrategy};
use chrono::NaiveDate;

/// Hysteresis helper: enter on `long_in`/`short_in`, exit on `long_out`/`short_out`.
fn band(
    pos: f64,
    long_in: bool,
    long_out: bool,
    short_in: bool,
    short_out: bool,
    side: Side,
) -> f64 {
    if pos > 0.0 {
        if long_out {
            0.0
        } else {
            pos
        }
    } else if pos < 0.0 {
        if short_out {
            0.0
        } else {
            pos
        }
    } else if long_in {
        1.0
    } else if short_in {
        side.apply(-1.0)
    } else {
        0.0
    }
}

struct RsiRevert {
    rsi: Rsi,
    lo: f64,
    hi: f64,
    trend: Option<Sma>,
    pos: f64,
}
impl SymbolStrategy for RsiRevert {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let up_trend = match &mut self.trend {
            Some(s) => s.update(b.close).map(|m| b.close > m),
            None => Some(true),
        };
        let (Some(r), Some(ok)) = (self.rsi.update(b.close), up_trend) else {
            return 0.0;
        };
        self.pos = band(
            self.pos,
            r < self.lo && ok,
            r > self.hi,
            false,
            false,
            Side::LongOnly,
        );
        self.pos
    }
}

struct Bollinger {
    w: Window,
    k: f64,
    pos: f64,
    side: Side,
    breakout: bool,
}
impl SymbolStrategy for Bollinger {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        self.w.push(b.close);
        if !self.w.full() {
            return 0.0;
        }
        let (m, sd) = (self.w.mean(), self.w.std());
        let (up, dn) = (m + self.k * sd, m - self.k * sd);
        let c = b.close;
        self.pos = if self.breakout {
            band(self.pos, c > up, c < m, c < dn, c > m, self.side)
        } else {
            band(self.pos, c < dn, c > m, c > up, c < m, self.side)
        };
        self.pos
    }
}

/// Internal bar strength: where the close sits in the day's range.
struct Ibs {
    lo: f64,
    hi: f64,
    pos: f64,
}
impl SymbolStrategy for Ibs {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let range = b.high - b.low;
        if range <= 0.0 {
            return self.pos;
        }
        let ibs = (b.close - b.low) / range;
        self.pos = band(
            self.pos,
            ibs < self.lo,
            ibs > self.hi,
            false,
            false,
            Side::LongOnly,
        );
        self.pos
    }
}

struct DownStreak {
    k: usize,
    streak: usize,
    prev: Option<f64>,
}
impl SymbolStrategy for DownStreak {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let Some(p) = self.prev.replace(b.close) else {
            return 0.0;
        };
        self.streak = if b.close < p { self.streak + 1 } else { 0 };
        if self.streak >= self.k {
            1.0
        } else {
            0.0
        }
    }
}

/// Stochastic %K (smoothed) / Williams %R family.
struct Stoch {
    hi: Window,
    lo: Window,
    smooth: Sma,
    lo_th: f64,
    hi_th: f64,
    pos: f64,
    side: Side,
}
impl SymbolStrategy for Stoch {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        self.hi.push(b.high);
        self.lo.push(b.low);
        if !self.hi.full() {
            return 0.0;
        }
        let (h, l) = (self.hi.max(), self.lo.min());
        let raw = if h > l {
            100.0 * (b.close - l) / (h - l)
        } else {
            50.0
        };
        let Some(k) = self.smooth.update(raw) else {
            return 0.0;
        };
        self.pos = band(
            self.pos,
            k < self.lo_th,
            k > 50.0,
            k > self.hi_th,
            k < 50.0,
            self.side,
        );
        self.pos
    }
}

struct Cci {
    tp: Window,
    trend: bool,
    pos: f64,
    side: Side,
}
impl SymbolStrategy for Cci {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let tp = (b.high + b.low + b.close) / 3.0;
        self.tp.push(tp);
        if !self.tp.full() {
            return 0.0;
        }
        let m = self.tp.mean();
        let md = self.tp.iter().map(|x| (x - m).abs()).sum::<f64>() / self.tp.len() as f64;
        if md == 0.0 {
            return self.pos;
        }
        let cci = (tp - m) / (0.015 * md);
        self.pos = if self.trend {
            band(
                self.pos,
                cci > 100.0,
                cci < 0.0,
                cci < -100.0,
                cci > 0.0,
                self.side,
            )
        } else {
            band(
                self.pos,
                cci < -100.0,
                cci > 0.0,
                cci > 100.0,
                cci < 0.0,
                self.side,
            )
        };
        self.pos
    }
}

/// Short-horizon return z-score: fade extreme multi-day moves.
struct ReturnZ {
    closes: Window,
    rets: Window,
    entry: f64,
    pos: f64,
    side: Side,
}
impl SymbolStrategy for ReturnZ {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        self.closes.push(b.close);
        if !self.closes.full() {
            return 0.0;
        }
        let r = (b.close / self.closes.first()).ln();
        self.rets.push(r);
        if !self.rets.full() || self.rets.std() == 0.0 {
            return 0.0;
        }
        let z = (r - self.rets.mean()) / self.rets.std();
        self.pos = band(
            self.pos,
            z < -self.entry,
            z > 0.0,
            z > self.entry,
            z < 0.0,
            self.side,
        );
        self.pos
    }
}

pub fn specs() -> Vec<Spec> {
    let mut v = Vec::new();
    for (n, lo, hi) in [
        (2, 10.0, 70.0),
        (2, 5.0, 60.0),
        (3, 15.0, 70.0),
        (5, 25.0, 60.0),
        (14, 30.0, 55.0),
        (14, 25.0, 70.0),
    ] {
        for filt in [false, true] {
            let tag = if filt { "sma200" } else { "nofilter" };
            v.push(Spec::per_symbol(
                format!("mean_reversion.rsi.{n}.{lo}.{hi}.{tag}"),
                "mean_reversion",
                format!(
                    "Buy when RSI({n}) < {lo}, sell when > {hi}{}.",
                    if filt {
                        ", only above the 200-day SMA (Connors-style)"
                    } else {
                        ""
                    }
                ),
                move || RsiRevert {
                    rsi: Rsi::new(n),
                    lo,
                    hi,
                    trend: filt.then(|| Sma::new(200)),
                    pos: 0.0,
                },
            ));
        }
    }
    for side in Side::BOTH {
        let (st, sw) = (side.tag(), side.words());
        for n in [10, 20, 50] {
            for k in [1.5, 2.0, 2.5] {
                v.push(Spec::per_symbol(
                    format!("mean_reversion.bollinger.{n}.{k}.{st}"),
                    "mean_reversion",
                    format!("{sw}: fade closes outside {k}-sigma {n}-day Bollinger bands, exit at the mean."),
                    move || Bollinger { w: Window::new(n), k, pos: 0.0, side, breakout: false },
                ));
            }
        }
        for n in [20, 50] {
            for k in [1.0, 2.0] {
                v.push(Spec::per_symbol(
                    format!("trend.bollinger_breakout.{n}.{k}.{st}"),
                    "trend",
                    format!("{sw}: follow closes outside {k}-sigma {n}-day Bollinger bands, exit at the mean."),
                    move || Bollinger { w: Window::new(n), k, pos: 0.0, side, breakout: true },
                ));
            }
        }
        for (n, s, lo, hi, name) in [
            (14, 3, 20.0, 80.0, "stoch"),
            (5, 3, 10.0, 90.0, "stoch"),
            (2, 1, 10.0, 90.0, "williams_r"),
            (10, 1, 10.0, 90.0, "williams_r"),
        ] {
            v.push(Spec::per_symbol(
                format!("mean_reversion.{name}.{n}.{s}.{st}"),
                "mean_reversion",
                format!("{sw}: fade %K({n}, smooth {s}) beyond {lo}/{hi}, exit at 50."),
                move || Stoch {
                    hi: Window::new(n),
                    lo: Window::new(n),
                    smooth: Sma::new(s),
                    lo_th: lo,
                    hi_th: hi,
                    pos: 0.0,
                    side,
                },
            ));
        }
        for n in [14, 20, 40] {
            v.push(Spec::per_symbol(
                format!("mean_reversion.cci.{n}.{st}"),
                "mean_reversion",
                format!("{sw}: fade CCI({n}) beyond +/-100, exit at 0."),
                move || Cci {
                    tp: Window::new(n),
                    trend: false,
                    pos: 0.0,
                    side,
                },
            ));
            v.push(Spec::per_symbol(
                format!("trend.cci.{n}.{st}"),
                "trend",
                format!("{sw}: follow CCI({n}) beyond +/-100, exit at 0."),
                move || Cci {
                    tp: Window::new(n),
                    trend: true,
                    pos: 0.0,
                    side,
                },
            ));
        }
        for h in [3, 5, 10] {
            for e in [1.5, 2.0] {
                v.push(Spec::per_symbol(
                    format!("mean_reversion.return_z.{h}.{e}.{st}"),
                    "mean_reversion",
                    format!(
                        "{sw}: fade {h}-day returns more than {e} sigma from their 1-year norm."
                    ),
                    move || ReturnZ {
                        closes: Window::new(h + 1),
                        rets: Window::new(252),
                        entry: e,
                        pos: 0.0,
                        side,
                    },
                ));
            }
        }
    }
    for (lo, hi) in [(0.2, 0.7), (0.1, 0.5), (0.25, 0.8)] {
        v.push(Spec::per_symbol(
            format!("mean_reversion.ibs.{lo}.{hi}"),
            "mean_reversion",
            format!(
                "Buy when the close is in the bottom {:.0}% of the day's range, sell above {:.0}%.",
                lo * 100.0,
                hi * 100.0
            ),
            move || Ibs { lo, hi, pos: 0.0 },
        ));
    }
    for k in [2, 3, 4, 5] {
        v.push(Spec::per_symbol(
            format!("mean_reversion.down_streak.{k}"),
            "mean_reversion",
            format!("Buy after {k} consecutive down closes; sell on the first up close."),
            move || DownStreak {
                k,
                streak: 0,
                prev: None,
            },
        ));
    }
    v
}
