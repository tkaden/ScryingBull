//! Trend following: buy strength, sell weakness.

use crate::data::Bar;
use crate::indicators::{linreg, Atr, Ema, Ma, MaKind, Sma, Window};
use crate::strategy::{Side, Spec, SymbolStrategy};
use chrono::NaiveDate;

fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

struct MaCross {
    fast: Ma,
    slow: Ma,
    side: Side,
}
impl SymbolStrategy for MaCross {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        match (self.fast.update(b.close), self.slow.update(b.close)) {
            (Some(f), Some(s)) => self.side.apply(sign(f - s)),
            _ => 0.0,
        }
    }
}

struct PriceVsMa {
    ma: Ma,
    side: Side,
}
impl SymbolStrategy for PriceVsMa {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        self.ma
            .update(b.close)
            .map_or(0.0, |m| self.side.apply(sign(b.close - m)))
    }
}

struct TripleEma {
    a: Ema,
    b: Ema,
    c: Ema,
    side: Side,
}
impl SymbolStrategy for TripleEma {
    fn next(&mut self, _: NaiveDate, bar: &Bar) -> f64 {
        match (
            self.a.update(bar.close),
            self.b.update(bar.close),
            self.c.update(bar.close),
        ) {
            (Some(f), Some(m), Some(s)) if f > m && m > s => 1.0,
            (Some(f), Some(m), Some(s)) if f < m && m < s => self.side.apply(-1.0),
            _ => 0.0,
        }
    }
}

/// Turtle-style channel breakout.
struct Donchian {
    hi: Window,
    lo: Window,
    exit_hi: Window,
    exit_lo: Window,
    pos: f64,
    side: Side,
}
impl SymbolStrategy for Donchian {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        if self.hi.full() {
            if b.close > self.hi.max() {
                self.pos = 1.0;
            } else if b.close < self.lo.min() {
                self.pos = self.side.apply(-1.0);
            } else if (self.pos > 0.0 && b.close < self.exit_lo.min())
                || (self.pos < 0.0 && b.close > self.exit_hi.max())
            {
                self.pos = 0.0;
            }
        }
        // Channels use *prior* bars, so update after the decision.
        self.hi.push(b.high);
        self.lo.push(b.low);
        self.exit_hi.push(b.high);
        self.exit_lo.push(b.low);
        self.pos
    }
}

struct Macd {
    fast: Ema,
    slow: Ema,
    signal: Ema,
    side: Side,
}
impl SymbolStrategy for Macd {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let (Some(f), Some(s)) = (self.fast.update(b.close), self.slow.update(b.close)) else {
            return 0.0;
        };
        let m = f - s;
        self.signal
            .update(m)
            .map_or(0.0, |sig| self.side.apply(sign(m - sig)))
    }
}

struct Supertrend {
    atr: Atr,
    mult: f64,
    upper: f64,
    lower: f64,
    dir: f64,
    prev_close: f64,
    side: Side,
}
impl SymbolStrategy for Supertrend {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let Some(atr) = self.atr.update(b.high, b.low, b.close) else {
            self.prev_close = b.close;
            return 0.0;
        };
        let mid = (b.high + b.low) / 2.0;
        let (bu, bl) = (mid + self.mult * atr, mid - self.mult * atr);
        if self.dir == 0.0 {
            (self.upper, self.lower, self.dir) = (bu, bl, 1.0);
        }
        self.upper = if bu < self.upper || self.prev_close > self.upper {
            bu
        } else {
            self.upper
        };
        self.lower = if bl > self.lower || self.prev_close < self.lower {
            bl
        } else {
            self.lower
        };
        if b.close > self.upper {
            self.dir = 1.0;
        } else if b.close < self.lower {
            self.dir = -1.0;
        }
        self.prev_close = b.close;
        self.side.apply(self.dir)
    }
}

/// Trade only when the regression slope of log price is statistically strong.
struct LinRegSlope {
    w: Window,
    t_min: f64,
    side: Side,
}
impl SymbolStrategy for LinRegSlope {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        self.w.push(b.close.ln());
        if !self.w.full() {
            return 0.0;
        }
        let (_, t) = linreg(&self.w).unwrap_or((0.0, 0.0));
        if t > self.t_min {
            1.0
        } else if t < -self.t_min {
            self.side.apply(-1.0)
        } else {
            0.0
        }
    }
}

struct Aroon {
    hi: Window,
    lo: Window,
    side: Side,
}
impl SymbolStrategy for Aroon {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        self.hi.push(b.high);
        self.lo.push(b.low);
        if !self.hi.full() {
            return 0.0;
        }
        let n = self.hi.capacity() as f64;
        let argmax = self
            .hi
            .iter()
            .rev()
            .enumerate()
            .fold((0, f64::MIN), |a, (k, &x)| if x > a.1 { (k, x) } else { a })
            .0;
        let argmin = self
            .lo
            .iter()
            .rev()
            .enumerate()
            .fold((0, f64::MAX), |a, (k, &x)| if x < a.1 { (k, x) } else { a })
            .0;
        let osc = (argmin as f64 - argmax as f64) / n * 100.0; // up - down
        if osc > 50.0 {
            1.0
        } else if osc < -50.0 {
            self.side.apply(-1.0)
        } else {
            0.0
        }
    }
}

/// Ichimoku cloud: long above the cloud with tenkan > kijun.
struct Ichimoku {
    h9: Window,
    l9: Window,
    h26: Window,
    l26: Window,
    h52: Window,
    l52: Window,
    /// (senkou A, senkou B) computed each day; the cloud "now" is the value from 26 days ago.
    cloud: Window,
    cloud_b: Window,
    side: Side,
}
impl SymbolStrategy for Ichimoku {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        for (h, l) in [
            (&mut self.h9, &mut self.l9),
            (&mut self.h26, &mut self.l26),
            (&mut self.h52, &mut self.l52),
        ] {
            h.push(b.high);
            l.push(b.low);
        }
        if !self.h52.full() {
            return 0.0;
        }
        let tenkan = (self.h9.max() + self.l9.min()) / 2.0;
        let kijun = (self.h26.max() + self.l26.min()) / 2.0;
        self.cloud.push((tenkan + kijun) / 2.0);
        self.cloud_b.push((self.h52.max() + self.l52.min()) / 2.0);
        if !self.cloud.full() {
            return 0.0;
        }
        let (a, bb) = (self.cloud.first(), self.cloud_b.first());
        if b.close > a.max(bb) && tenkan > kijun {
            1.0
        } else if b.close < a.min(bb) && tenkan < kijun {
            self.side.apply(-1.0)
        } else {
            0.0
        }
    }
}

/// Hull moving average slope.
struct Hull {
    half: Window,
    full: Window,
    smooth: Window,
    prev: Option<f64>,
    side: Side,
}
fn wma(w: &Window) -> f64 {
    let (mut num, mut den) = (0.0, 0.0);
    for (k, x) in w.iter().enumerate() {
        num += (k + 1) as f64 * x;
        den += (k + 1) as f64;
    }
    num / den
}
impl SymbolStrategy for Hull {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        self.half.push(b.close);
        self.full.push(b.close);
        if !self.full.full() {
            return 0.0;
        }
        self.smooth.push(2.0 * wma(&self.half) - wma(&self.full));
        if !self.smooth.full() {
            return 0.0;
        }
        let h = wma(&self.smooth);
        let out = self.prev.map_or(0.0, |p| self.side.apply(sign(h - p)));
        self.prev = Some(h);
        out
    }
}

/// Pring's Know Sure Thing: weighted sum of smoothed rates of change.
struct Kst {
    closes: Window,
    rocs: [Sma; 4],
    signal: Sma,
    side: Side,
}
impl SymbolStrategy for Kst {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        self.closes.push(b.close);
        if !self.closes.full() {
            return 0.0;
        }
        let mut kst = 0.0;
        let mut ready = true;
        for (k, (lag, sma)) in [10usize, 15, 20, 30]
            .iter()
            .zip(self.rocs.iter_mut())
            .enumerate()
        {
            let roc = b.close / self.closes.ago(*lag) - 1.0;
            match sma.update(roc) {
                Some(v) => kst += (k + 1) as f64 * v,
                None => ready = false,
            }
        }
        if !ready {
            return 0.0;
        }
        self.signal
            .update(kst)
            .map_or(0.0, |s| self.side.apply(sign(kst - s)))
    }
}

pub fn specs() -> Vec<Spec> {
    let mut v = Vec::new();
    for side in Side::BOTH {
        let (st, sw) = (side.tag(), side.words());
        for kind in [MaKind::Sma, MaKind::Ema] {
            for (f, s) in [(5, 20), (10, 30), (10, 50), (20, 60), (20, 100), (50, 200)] {
                v.push(Spec::per_symbol(
                    format!("trend.ma_cross.{}.{f}x{s}.{st}", kind.tag()),
                    "trend",
                    format!("{sw}: {} {f} vs {s} crossover.", kind.tag().to_uppercase()),
                    move || MaCross {
                        fast: Ma::new(kind, f),
                        slow: Ma::new(kind, s),
                        side,
                    },
                ));
            }
            for n in [20, 50, 100, 150, 200] {
                v.push(Spec::per_symbol(
                    format!("trend.price_vs_ma.{}.{n}.{st}", kind.tag()),
                    "trend",
                    format!(
                        "{sw}: close above/below its {n}-day {}.",
                        kind.tag().to_uppercase()
                    ),
                    move || PriceVsMa {
                        ma: Ma::new(kind, n),
                        side,
                    },
                ));
            }
        }
        for (a, b, c) in [(5, 20, 60), (10, 30, 120), (20, 60, 200)] {
            v.push(Spec::per_symbol(
                format!("trend.triple_ema.{a}.{b}.{c}.{st}"),
                "trend",
                format!("{sw}: EMAs {a}/{b}/{c} stacked in order."),
                move || TripleEma {
                    a: Ema::new(a),
                    b: Ema::new(b),
                    c: Ema::new(c),
                    side,
                },
            ));
        }
        for (e, x) in [(20, 10), (55, 20), (100, 50), (200, 100)] {
            v.push(Spec::per_symbol(
                format!("trend.donchian.{e}.{x}.{st}"),
                "trend",
                format!(
                    "{sw}: break of {e}-day channel, exit on {x}-day opposite channel (Turtle)."
                ),
                move || Donchian {
                    hi: Window::new(e),
                    lo: Window::new(e),
                    exit_hi: Window::new(x),
                    exit_lo: Window::new(x),
                    pos: 0.0,
                    side,
                },
            ));
        }
        for (f, s, g) in [(12, 26, 9), (8, 17, 9), (5, 35, 5), (19, 39, 9)] {
            v.push(Spec::per_symbol(
                format!("trend.macd.{f}.{s}.{g}.{st}"),
                "trend",
                format!("{sw}: MACD({f},{s}) vs its {g}-day signal line."),
                move || Macd {
                    fast: Ema::new(f),
                    slow: Ema::new(s),
                    signal: Ema::new(g),
                    side,
                },
            ));
        }
        for n in [10, 20] {
            for m in [2.0, 3.0, 4.0] {
                v.push(Spec::per_symbol(
                    format!("trend.supertrend.{n}.{m}.{st}"),
                    "trend",
                    format!("{sw}: Supertrend with ATR({n}) x {m}."),
                    move || Supertrend {
                        atr: Atr::new(n),
                        mult: m,
                        upper: 0.0,
                        lower: 0.0,
                        dir: 0.0,
                        prev_close: 0.0,
                        side,
                    },
                ));
            }
        }
        for n in [20, 60, 120] {
            for t in [0.0, 2.0, 4.0] {
                v.push(Spec::per_symbol(
                    format!("trend.linreg_t.{n}.{t}.{st}"),
                    "trend",
                    format!(
                        "{sw}: trade when the {n}-day log-price regression slope has |t| > {t}."
                    ),
                    move || LinRegSlope {
                        w: Window::new(n),
                        t_min: t,
                        side,
                    },
                ));
            }
        }
        for n in [25, 50] {
            v.push(Spec::per_symbol(
                format!("trend.aroon.{n}.{st}"),
                "trend",
                format!("{sw}: Aroon({n}) oscillator beyond +/-50."),
                move || Aroon {
                    hi: Window::new(n + 1),
                    lo: Window::new(n + 1),
                    side,
                },
            ));
        }
        v.push(Spec::per_symbol(
            format!("trend.ichimoku.{st}"),
            "trend",
            format!("{sw}: Ichimoku (9/26/52) price vs cloud with tenkan/kijun confirmation."),
            move || Ichimoku {
                h9: Window::new(9),
                l9: Window::new(9),
                h26: Window::new(26),
                l26: Window::new(26),
                h52: Window::new(52),
                l52: Window::new(52),
                cloud: Window::new(27),
                cloud_b: Window::new(27),
                side,
            },
        ));
        for n in [20, 55] {
            v.push(Spec::per_symbol(
                format!("trend.hull.{n}.{st}"),
                "trend",
                format!("{sw}: direction of the Hull moving average ({n})."),
                move || Hull {
                    half: Window::new(n / 2),
                    full: Window::new(n),
                    smooth: Window::new((n as f64).sqrt().round() as usize),
                    prev: None,
                    side,
                },
            ));
        }
        v.push(Spec::per_symbol(
            format!("trend.kst.{st}"),
            "trend",
            format!("{sw}: Know Sure Thing vs its 9-day signal."),
            move || Kst {
                closes: Window::new(31),
                rocs: [Sma::new(10), Sma::new(10), Sma::new(10), Sma::new(15)],
                signal: Sma::new(9),
                side,
            },
        ));
    }
    v
}
