//! Incremental (streaming) indicators. Each `update` consumes one new value and
//! returns `Some(value)` once warmed up. Being incremental, they cannot look ahead.

use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct Window {
    n: usize,
    buf: VecDeque<f64>,
    sum: f64,
    sumsq: f64,
}

impl Window {
    pub fn new(n: usize) -> Self {
        Window {
            n: n.max(1),
            buf: VecDeque::with_capacity(n + 1),
            sum: 0.0,
            sumsq: 0.0,
        }
    }
    pub fn push(&mut self, x: f64) {
        self.buf.push_back(x);
        self.sum += x;
        self.sumsq += x * x;
        if self.buf.len() > self.n {
            let y = self.buf.pop_front().unwrap();
            self.sum -= y;
            self.sumsq -= y * y;
        }
    }
    pub fn full(&self) -> bool {
        self.buf.len() == self.n
    }
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
    pub fn capacity(&self) -> usize {
        self.n
    }
    pub fn mean(&self) -> f64 {
        self.sum / self.buf.len().max(1) as f64
    }
    pub fn std(&self) -> f64 {
        let k = self.buf.len() as f64;
        if k < 2.0 {
            return 0.0;
        }
        ((self.sumsq - self.sum * self.sum / k) / (k - 1.0))
            .max(0.0)
            .sqrt()
    }
    pub fn max(&self) -> f64 {
        self.buf.iter().copied().fold(f64::NEG_INFINITY, f64::max)
    }
    pub fn min(&self) -> f64 {
        self.buf.iter().copied().fold(f64::INFINITY, f64::min)
    }
    pub fn first(&self) -> f64 {
        self.buf.front().copied().unwrap_or(f64::NAN)
    }
    pub fn last(&self) -> f64 {
        self.buf.back().copied().unwrap_or(f64::NAN)
    }
    /// k-th most recent value (0 = latest).
    pub fn ago(&self, k: usize) -> f64 {
        self.buf
            .len()
            .checked_sub(k + 1)
            .map_or(f64::NAN, |i| self.buf[i])
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &f64> + ExactSizeIterator {
        self.buf.iter()
    }
}

#[derive(Clone, Debug)]
pub struct Sma(Window);
impl Sma {
    pub fn new(n: usize) -> Self {
        Sma(Window::new(n))
    }
    pub fn update(&mut self, x: f64) -> Option<f64> {
        self.0.push(x);
        self.0.full().then(|| self.0.mean())
    }
}

#[derive(Clone, Debug)]
pub struct Ema {
    alpha: f64,
    n: usize,
    seen: usize,
    value: f64,
}
impl Ema {
    pub fn new(n: usize) -> Self {
        Self::with_alpha(2.0 / (n as f64 + 1.0), n)
    }
    /// Wilder's smoothing (used by RSI/ATR).
    pub fn wilder(n: usize) -> Self {
        Self::with_alpha(1.0 / n as f64, n)
    }
    fn with_alpha(alpha: f64, n: usize) -> Self {
        Ema {
            alpha,
            n: n.max(1),
            seen: 0,
            value: 0.0,
        }
    }
    pub fn update(&mut self, x: f64) -> Option<f64> {
        self.seen += 1;
        if self.seen == 1 {
            self.value = x;
        } else if self.seen <= self.n {
            // Simple average during warm-up gives a sane seed.
            self.value += (x - self.value) / self.seen as f64;
        } else {
            self.value += self.alpha * (x - self.value);
        }
        (self.seen >= self.n).then_some(self.value)
    }
    pub fn value(&self) -> Option<f64> {
        (self.seen >= self.n).then_some(self.value)
    }
}

/// SMA or EMA chosen at runtime.
#[derive(Clone, Debug)]
pub enum Ma {
    S(Sma),
    E(Ema),
}
impl Ma {
    pub fn new(kind: MaKind, n: usize) -> Self {
        match kind {
            MaKind::Sma => Ma::S(Sma::new(n)),
            MaKind::Ema => Ma::E(Ema::new(n)),
        }
    }
    pub fn update(&mut self, x: f64) -> Option<f64> {
        match self {
            Ma::S(m) => m.update(x),
            Ma::E(m) => m.update(x),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum MaKind {
    Sma,
    Ema,
}
impl MaKind {
    pub fn tag(self) -> &'static str {
        match self {
            MaKind::Sma => "sma",
            MaKind::Ema => "ema",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Rsi {
    gain: Ema,
    loss: Ema,
    prev: Option<f64>,
}
impl Rsi {
    pub fn new(n: usize) -> Self {
        Rsi {
            gain: Ema::wilder(n),
            loss: Ema::wilder(n),
            prev: None,
        }
    }
    pub fn update(&mut self, close: f64) -> Option<f64> {
        let prev = self.prev.replace(close)?;
        let d = close - prev;
        let g = self.gain.update(d.max(0.0))?;
        let l = self.loss.update((-d).max(0.0))?;
        Some(if l == 0.0 {
            100.0
        } else {
            100.0 - 100.0 / (1.0 + g / l)
        })
    }
}

#[derive(Clone, Debug)]
pub struct Atr {
    ema: Ema,
    prev_close: Option<f64>,
}
impl Atr {
    pub fn new(n: usize) -> Self {
        Atr {
            ema: Ema::wilder(n),
            prev_close: None,
        }
    }
    pub fn update(&mut self, high: f64, low: f64, close: f64) -> Option<f64> {
        let tr = match self.prev_close {
            Some(pc) => (high - low).max((high - pc).abs()).max((low - pc).abs()),
            None => high - low,
        };
        self.prev_close = Some(close);
        self.ema.update(tr)
    }
}

/// Rolling standard deviation of log returns.
#[derive(Clone, Debug)]
pub struct RetVol {
    w: Window,
    prev: Option<f64>,
}
impl RetVol {
    pub fn new(n: usize) -> Self {
        RetVol {
            w: Window::new(n),
            prev: None,
        }
    }
    pub fn update(&mut self, close: f64) -> Option<f64> {
        let prev = self.prev.replace(close)?;
        self.w.push((close / prev).ln());
        self.w.full().then(|| self.w.std())
    }
}

/// Ordinary least squares of y on x = 0..n-1 over a rolling window.
/// Returns (slope, t-statistic of slope).
pub fn linreg(ys: &Window) -> Option<(f64, f64)> {
    let n = ys.len();
    if n < 3 {
        return None;
    }
    let nf = n as f64;
    let xm = (nf - 1.0) / 2.0;
    let ym = ys.mean();
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (i, y) in ys.iter().enumerate() {
        let dx = i as f64 - xm;
        sxy += dx * (y - ym);
        sxx += dx * dx;
    }
    let slope = sxy / sxx;
    let mut sse = 0.0;
    for (i, y) in ys.iter().enumerate() {
        let e = y - (ym + slope * (i as f64 - xm));
        sse += e * e;
    }
    let se = (sse / (nf - 2.0) / sxx).sqrt();
    Some((slope, if se > 0.0 { slope / se } else { 0.0 }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sma_and_window() {
        let mut s = Sma::new(3);
        assert_eq!(s.update(1.0), None);
        assert_eq!(s.update(2.0), None);
        assert_eq!(s.update(3.0), Some(2.0));
        assert_eq!(s.update(4.0), Some(3.0));
        let mut w = Window::new(3);
        for x in [1.0, 2.0, 3.0, 4.0] {
            w.push(x);
        }
        assert_eq!((w.min(), w.max(), w.ago(0), w.ago(2)), (2.0, 4.0, 4.0, 2.0));
        assert!((w.std() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn rsi_extremes() {
        let mut r = Rsi::new(5);
        let mut last = None;
        for i in 0..20 {
            last = r.update(100.0 + i as f64);
        }
        assert_eq!(last, Some(100.0));
    }

    #[test]
    fn linreg_perfect_line() {
        let mut w = Window::new(10);
        for i in 0..10 {
            w.push(2.0 * i as f64 + 1.0);
        }
        let (slope, _) = linreg(&w).unwrap();
        assert!((slope - 2.0).abs() < 1e-12);
    }
}
