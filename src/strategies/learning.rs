//! Online machine learning. Every model learns strictly from the past: it is
//! trained on yesterday's features against today's outcome, then predicts tomorrow.

use crate::data::Bar;
use crate::indicators::{Rsi, Sma, Window};
use crate::strategy::{Side, Spec, SymbolStrategy};
use chrono::NaiveDate;
use std::collections::VecDeque;

const NF: usize = 9;

/// Normalised technical features of the price stream.
struct Features {
    rets: Window,
    vol: Window,
    sma20: Sma,
    sma50: Sma,
    rsi: Rsi,
    prev: Option<f64>,
}
impl Features {
    fn new() -> Self {
        Features {
            rets: Window::new(5),
            vol: Window::new(20),
            sma20: Sma::new(20),
            sma50: Sma::new(50),
            rsi: Rsi::new(14),
            prev: None,
        }
    }
    /// Returns (today's log return, features) once warm.
    fn update(&mut self, close: f64) -> (Option<f64>, Option<[f64; NF]>) {
        let ret = self.prev.replace(close).map(|p| (close / p).ln());
        if let Some(r) = ret {
            self.rets.push(r);
            self.vol.push(r);
        }
        let (s20, s50, rsi) = (
            self.sma20.update(close),
            self.sma50.update(close),
            self.rsi.update(close),
        );
        let (Some(s20), Some(s50), Some(rsi)) = (s20, s50, rsi) else {
            return (ret, None);
        };
        let sd = self.vol.std().max(1e-4);
        let mut f = [0.0; NF];
        for (k, r) in self.rets.iter().rev().enumerate() {
            f[k] = (r / sd).clamp(-4.0, 4.0);
        }
        f[5] = ((close / s20).ln() / (sd * 20f64.sqrt())).clamp(-4.0, 4.0);
        f[6] = ((close / s50).ln() / (sd * 50f64.sqrt())).clamp(-4.0, 4.0);
        f[7] = (rsi - 50.0) / 25.0;
        f[8] = 1.0; // bias
        (ret, Some(f))
    }
}

fn dot(a: &[f64; NF], b: &[f64; NF]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

struct Logistic {
    feats: Features,
    w: [f64; NF],
    lr: f64,
    l2: f64,
    edge: f64,
    last: Option<[f64; NF]>,
    seen: usize,
    side: Side,
}
impl SymbolStrategy for Logistic {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let (ret, f) = self.feats.update(b.close);
        if let (Some(x), Some(r)) = (self.last, ret) {
            let y = if r > 0.0 { 1.0 } else { 0.0 };
            let p = 1.0 / (1.0 + (-dot(&self.w, &x)).exp());
            for k in 0..NF {
                self.w[k] += self.lr * ((y - p) * x[k] - self.l2 * self.w[k]);
            }
            self.seen += 1;
        }
        self.last = f;
        let Some(x) = f else { return 0.0 };
        if self.seen < 250 {
            return 0.0;
        }
        let p = 1.0 / (1.0 + (-dot(&self.w, &x)).exp());
        if p > 0.5 + self.edge {
            1.0
        } else if p < 0.5 - self.edge {
            self.side.apply(-1.0)
        } else {
            0.0
        }
    }
}

/// Averaged perceptron on the same features.
struct Perceptron {
    feats: Features,
    w: [f64; NF],
    avg: [f64; NF],
    last: Option<[f64; NF]>,
    seen: usize,
    side: Side,
}
impl SymbolStrategy for Perceptron {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let (ret, f) = self.feats.update(b.close);
        if let (Some(x), Some(r)) = (self.last, ret) {
            let y = if r > 0.0 { 1.0 } else { -1.0 };
            if y * dot(&self.w, &x) <= 0.0 {
                for k in 0..NF {
                    self.w[k] += y * x[k];
                }
            }
            for k in 0..NF {
                self.avg[k] += self.w[k];
            }
            self.seen += 1;
        }
        self.last = f;
        match f {
            Some(x) if self.seen >= 250 => self.side.apply(dot(&self.avg, &x).signum()),
            _ => 0.0,
        }
    }
}

/// k-nearest-neighbour analogues: find past moments whose last `m` (vol-normalised)
/// returns looked most like today's, and bet on what happened next on average.
struct Knn {
    rets: Window,
    vol: Window,
    k: usize,
    cap: usize,
    hist: VecDeque<(Vec<f64>, f64)>,
    last: Option<Vec<f64>>,
    prev: Option<f64>,
    side: Side,
}
impl SymbolStrategy for Knn {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let Some(p) = self.prev.replace(b.close) else {
            return 0.0;
        };
        let r = (b.close / p).ln();
        if let Some(pat) = self.last.take() {
            self.hist.push_back((pat, r));
            if self.hist.len() > self.cap {
                self.hist.pop_front();
            }
        }
        self.rets.push(r);
        self.vol.push(r);
        if !self.vol.full() {
            return 0.0;
        }
        let sd = self.vol.std().max(1e-4);
        let pat: Vec<f64> = self.rets.iter().map(|x| x / sd).collect();
        self.last = Some(pat.clone());
        if self.hist.len() < self.cap / 2 {
            return 0.0;
        }
        let mut d: Vec<(f64, f64)> = self
            .hist
            .iter()
            .map(|(h, nr)| {
                (
                    h.iter()
                        .zip(&pat)
                        .map(|(a, b)| (a - b).powi(2))
                        .sum::<f64>(),
                    *nr,
                )
            })
            .collect();
        let k = self.k.min(d.len());
        d.select_nth_unstable_by(k - 1, |a, b| a.0.total_cmp(&b.0));
        let pred: f64 = d[..k].iter().map(|x| x.1).sum::<f64>() / k as f64;
        self.side.apply(pred.signum())
    }
}

/// Markov chain over up/down sequences: P(up tomorrow | last `order` days' directions),
/// learned online with exponential forgetting.
struct Markov {
    order: usize,
    state: usize,
    days: usize,
    up: Vec<f64>,
    tot: Vec<f64>,
    decay: f64,
    edge: f64,
    prev: Option<f64>,
    side: Side,
}
impl SymbolStrategy for Markov {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let Some(p) = self.prev.replace(b.close) else {
            return 0.0;
        };
        let upday = b.close > p;
        let mask = (1 << self.order) - 1;
        if self.days >= self.order {
            let s = self.state;
            self.up[s] = self.up[s] * self.decay + if upday { 1.0 } else { 0.0 };
            self.tot[s] = self.tot[s] * self.decay + 1.0;
        }
        self.state = ((self.state << 1) | upday as usize) & mask;
        self.days += 1;
        if self.days < 250 {
            return 0.0;
        }
        let s = self.state;
        let pu = (self.up[s] + 1.0) / (self.tot[s] + 2.0);
        if pu > 0.5 + self.edge {
            1.0
        } else if pu < 0.5 - self.edge {
            self.side.apply(-1.0)
        } else {
            0.0
        }
    }
}

pub fn specs() -> Vec<Spec> {
    let mut v = Vec::new();
    for side in Side::BOTH {
        let (st, sw) = (side.tag(), side.words());
        for (lr, edge) in [(0.01, 0.0), (0.01, 0.02), (0.05, 0.02)] {
            v.push(Spec::per_symbol(
                format!("learning.logistic.{lr}.{edge}.{st}"),
                "learning",
                format!("{sw}: online logistic regression (lr {lr}) on lagged returns, MA distance and RSI; trade when P(up) is {edge} from 0.5."),
                move || Logistic { feats: Features::new(), w: [0.0; NF], lr, l2: 1e-4, edge, last: None, seen: 0, side },
            ));
        }
        v.push(Spec::per_symbol(
            format!("learning.perceptron.{st}"),
            "learning",
            format!("{sw}: averaged perceptron on the same features."),
            move || Perceptron {
                feats: Features::new(),
                w: [0.0; NF],
                avg: [0.0; NF],
                last: None,
                seen: 0,
                side,
            },
        ));
        for (m, k) in [(5, 10), (5, 30), (10, 20)] {
            v.push(Spec::per_symbol(
                format!("learning.knn.{m}.{k}.{st}"),
                "learning",
                format!("{sw}: k-NN analogues - average next-day return of the {k} most similar past {m}-day return patterns."),
                move || Knn {
                    rets: Window::new(m),
                    vol: Window::new(60),
                    k,
                    cap: 500,
                    hist: VecDeque::new(),
                    last: None,
                    prev: None,
                    side,
                },
            ));
        }
        for order in [2, 3, 4, 5] {
            v.push(Spec::per_symbol(
                format!("learning.markov.{order}.{st}"),
                "learning",
                format!("{sw}: Markov chain on the last {order} up/down days, P(up) learned online with forgetting."),
                move || Markov {
                    order,
                    state: 0,
                    days: 0,
                    up: vec![0.0; 1 << order],
                    tot: vec![0.0; 1 << order],
                    decay: 0.998,
                    edge: 0.03,
                    prev: None,
                    side,
                },
            ));
        }
    }
    v
}
