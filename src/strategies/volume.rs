//! Volume-based methods: does money flow confirm the price move?

use crate::data::Bar;
use crate::indicators::{Ema, Sma, Window};
use crate::strategy::{Side, Spec, SymbolStrategy};
use chrono::NaiveDate;

/// Klinger volume oscillator vs its signal line (ported from the original Python repo).
struct Klinger {
    fast: Ema,
    slow: Ema,
    signal: Ema,
    prev_hlc3: Option<f64>,
    side: Side,
}
impl SymbolStrategy for Klinger {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let hlc3 = (b.high + b.low + b.close) / 3.0;
        let Some(prev) = self.prev_hlc3.replace(hlc3) else {
            return 0.0;
        };
        let sv = if hlc3 >= prev { b.volume } else { -b.volume };
        let (Some(f), Some(s)) = (self.fast.update(sv), self.slow.update(sv)) else {
            return 0.0;
        };
        let kvo = f - s;
        self.signal.update(kvo).map_or(0.0, |sig| {
            self.side.apply(if kvo > sig { 1.0 } else { -1.0 })
        })
    }
}

struct ObvTrend {
    obv: f64,
    ma: Sma,
    prev: Option<f64>,
    side: Side,
}
impl SymbolStrategy for ObvTrend {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        if let Some(p) = self.prev.replace(b.close) {
            self.obv += if b.close > p {
                b.volume
            } else if b.close < p {
                -b.volume
            } else {
                0.0
            };
        }
        self.ma.update(self.obv).map_or(0.0, |m| {
            self.side.apply(if self.obv > m { 1.0 } else { -1.0 })
        })
    }
}

/// Chaikin money flow.
struct Cmf {
    mfv: Window,
    vol: Window,
    thresh: f64,
    side: Side,
}
impl SymbolStrategy for Cmf {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let r = b.high - b.low;
        let mult = if r > 0.0 {
            ((b.close - b.low) - (b.high - b.close)) / r
        } else {
            0.0
        };
        self.mfv.push(mult * b.volume);
        self.vol.push(b.volume);
        if !self.mfv.full() || self.vol.mean() == 0.0 {
            return 0.0;
        }
        let cmf = self.mfv.mean() / self.vol.mean();
        if cmf > self.thresh {
            1.0
        } else if cmf < -self.thresh {
            self.side.apply(-1.0)
        } else {
            0.0
        }
    }
}

/// Money flow index: a volume-weighted RSI, used for mean reversion.
struct Mfi {
    pos_flow: Window,
    neg_flow: Window,
    prev_tp: Option<f64>,
    lo: f64,
    hi: f64,
    pos: f64,
}
impl SymbolStrategy for Mfi {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let tp = (b.high + b.low + b.close) / 3.0;
        let Some(p) = self.prev_tp.replace(tp) else {
            return 0.0;
        };
        let flow = tp * b.volume;
        self.pos_flow.push(if tp > p { flow } else { 0.0 });
        self.neg_flow.push(if tp < p { flow } else { 0.0 });
        if !self.pos_flow.full() {
            return 0.0;
        }
        let (pf, nf) = (self.pos_flow.mean(), self.neg_flow.mean());
        let mfi = if nf == 0.0 {
            100.0
        } else {
            100.0 - 100.0 / (1.0 + pf / nf)
        };
        if self.pos == 0.0 && mfi < self.lo {
            self.pos = 1.0;
        } else if self.pos > 0.0 && mfi > self.hi {
            self.pos = 0.0;
        }
        self.pos
    }
}

/// Unusual-volume up days: institutions accumulating? Hold for a while.
struct VolumeSpike {
    vol: Window,
    mult: f64,
    hold: usize,
    left: usize,
}
impl SymbolStrategy for VolumeSpike {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let spike = self.vol.full() && b.volume > self.mult * self.vol.mean() && b.close > b.open;
        self.vol.push(b.volume);
        self.left = self.left.saturating_sub(1);
        if spike {
            self.left = self.hold;
        }
        if self.left > 0 {
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
        for (f, s, g) in [(34, 55, 13), (20, 40, 9)] {
            v.push(Spec::per_symbol(
                format!("volume.klinger.{f}.{s}.{g}.{st}"),
                "volume",
                format!("{sw}: Klinger volume oscillator ({f}/{s}) vs {g}-day signal."),
                move || Klinger {
                    fast: Ema::new(f),
                    slow: Ema::new(s),
                    signal: Ema::new(g),
                    prev_hlc3: None,
                    side,
                },
            ));
        }
        for n in [20, 50] {
            v.push(Spec::per_symbol(
                format!("volume.obv.{n}.{st}"),
                "volume",
                format!("{sw}: on-balance volume above/below its {n}-day average."),
                move || ObvTrend {
                    obv: 0.0,
                    ma: Sma::new(n),
                    prev: None,
                    side,
                },
            ));
        }
        for t in [0.0, 0.05, 0.1] {
            v.push(Spec::per_symbol(
                format!("volume.cmf.20.{t}.{st}"),
                "volume",
                format!("{sw}: Chaikin money flow(20) beyond +/-{t}."),
                move || Cmf {
                    mfv: Window::new(20),
                    vol: Window::new(20),
                    thresh: t,
                    side,
                },
            ));
        }
    }
    for (lo, hi) in [(20.0, 60.0), (10.0, 50.0)] {
        v.push(Spec::per_symbol(
            format!("volume.mfi.14.{lo}.{hi}"),
            "volume",
            format!("Buy when money flow index(14) < {lo}, sell above {hi}."),
            move || Mfi {
                pos_flow: Window::new(14),
                neg_flow: Window::new(14),
                prev_tp: None,
                lo,
                hi,
                pos: 0.0,
            },
        ));
    }
    for (m, h) in [(2.0, 5), (2.0, 20), (3.0, 10)] {
        v.push(Spec::per_symbol(
            format!("volume.spike.{m}.{h}"),
            "volume",
            format!("After an up day on {m}x average volume, hold long for {h} days."),
            move || VolumeSpike {
                vol: Window::new(50),
                mult: m,
                hold: h,
                left: 0,
            },
        ));
    }
    v
}
