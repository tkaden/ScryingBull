//! Statistical models of the price process: filters, regime and memory estimators.

use crate::data::Bar;
use crate::indicators::Window;
use crate::strategy::{Side, Spec, SymbolStrategy};
use chrono::NaiveDate;

/// Local-linear-trend Kalman filter on log price; trade the sign of the estimated velocity.
struct KalmanTrend {
    x: [f64; 2],
    p: [[f64; 2]; 2],
    q: f64,
    r: f64,
    n: usize,
    side: Side,
}
impl SymbolStrategy for KalmanTrend {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let z = b.close.ln();
        self.n += 1;
        if self.n == 1 {
            self.x = [z, 0.0];
            self.p = [[1.0, 0.0], [0.0, 1.0]];
            return 0.0;
        }
        // Predict: x = F x, P = F P F' + Q with F = [[1,1],[0,1]].
        let [l, v] = self.x;
        let x = [l + v, v];
        let p = self.p;
        let mut pp = [
            [p[0][0] + p[0][1] + p[1][0] + p[1][1], p[0][1] + p[1][1]],
            [p[1][0] + p[1][1], p[1][1]],
        ];
        pp[0][0] += self.q;
        pp[1][1] += self.q * 0.01;
        // Update with H = [1, 0].
        let s = pp[0][0] + self.r;
        let k = [pp[0][0] / s, pp[1][0] / s];
        let y = z - x[0];
        self.x = [x[0] + k[0] * y, x[1] + k[1] * y];
        self.p = [
            [(1.0 - k[0]) * pp[0][0], (1.0 - k[0]) * pp[0][1]],
            [pp[1][0] - k[1] * pp[0][0], pp[1][1] - k[1] * pp[0][1]],
        ];
        if self.n < 30 {
            return 0.0;
        }
        self.side.apply(self.x[1].signum())
    }
}

/// Variance ratio VR(k) = Var(k-day returns) / (k * Var(1-day returns)).
/// VR > 1 suggests trending (follow), VR < 1 mean-reverting (fade).
struct VarianceRatio {
    rets: Window,
    k: usize,
    band: f64,
    prev: Option<f64>,
}
impl SymbolStrategy for VarianceRatio {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let Some(p) = self.prev.replace(b.close) else {
            return 0.0;
        };
        self.rets.push((b.close / p).ln());
        if !self.rets.full() {
            return 0.0;
        }
        let r: Vec<f64> = self.rets.iter().copied().collect();
        let v1 = self.rets.std().powi(2);
        let kr: Vec<f64> = r.windows(self.k).map(|w| w.iter().sum()).collect();
        let m = kr.iter().sum::<f64>() / kr.len() as f64;
        let vk = kr.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (kr.len() - 1) as f64;
        if v1 == 0.0 {
            return 0.0;
        }
        let vr = vk / (self.k as f64 * v1);
        let recent: f64 = r[r.len() - self.k..].iter().sum();
        if vr > 1.0 + self.band {
            recent.signum().max(0.0)
        } else if vr < 1.0 - self.band {
            (-recent).signum().max(0.0)
        } else {
            0.0
        }
    }
}

/// Hurst exponent via rescaled range on the trailing window; trend-follow when H > 0.5+band,
/// fade when H < 0.5-band.
struct Hurst {
    rets: Window,
    band: f64,
    prev: Option<f64>,
}
fn rescaled_range(x: &[f64]) -> f64 {
    let m = x.iter().sum::<f64>() / x.len() as f64;
    let (mut cum, mut hi, mut lo, mut ss) = (0.0f64, 0.0f64, 0.0f64, 0.0);
    for v in x {
        cum += v - m;
        hi = hi.max(cum);
        lo = lo.min(cum);
        ss += (v - m).powi(2);
    }
    let sd = (ss / x.len() as f64).sqrt();
    if sd > 0.0 {
        (hi - lo) / sd
    } else {
        0.0
    }
}
impl SymbolStrategy for Hurst {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let Some(p) = self.prev.replace(b.close) else {
            return 0.0;
        };
        self.rets.push((b.close / p).ln());
        if !self.rets.full() {
            return 0.0;
        }
        let r: Vec<f64> = self.rets.iter().copied().collect();
        // Regress log(R/S) on log(chunk size).
        let mut pts = Vec::new();
        let mut size = 8;
        while size <= r.len() / 2 {
            let rs: Vec<f64> = r
                .chunks_exact(size)
                .map(rescaled_range)
                .filter(|v| *v > 0.0)
                .collect();
            if !rs.is_empty() {
                pts.push((
                    (size as f64).ln(),
                    (rs.iter().sum::<f64>() / rs.len() as f64).ln(),
                ));
            }
            size *= 2;
        }
        if pts.len() < 3 {
            return 0.0;
        }
        let n = pts.len() as f64;
        let (mx, my) = (
            pts.iter().map(|p| p.0).sum::<f64>() / n,
            pts.iter().map(|p| p.1).sum::<f64>() / n,
        );
        let h = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>()
            / pts.iter().map(|p| (p.0 - mx).powi(2)).sum::<f64>();
        let recent: f64 = r[r.len() - 10..].iter().sum();
        if h > 0.5 + self.band {
            recent.signum().max(0.0)
        } else if h < 0.5 - self.band {
            (-recent).signum().max(0.0)
        } else {
            0.0
        }
    }
}

/// Lag-1 autocorrelation: if recent returns persist, bet on continuation; if they alternate, on reversal.
struct Autocorr {
    rets: Window,
    min_abs: f64,
    prev: Option<f64>,
    side: Side,
}
impl SymbolStrategy for Autocorr {
    fn next(&mut self, _: NaiveDate, b: &Bar) -> f64 {
        let Some(p) = self.prev.replace(b.close) else {
            return 0.0;
        };
        self.rets.push((b.close / p).ln());
        if !self.rets.full() {
            return 0.0;
        }
        let m = self.rets.mean();
        let r: Vec<f64> = self.rets.iter().map(|x| x - m).collect();
        let var: f64 = r.iter().map(|x| x * x).sum();
        if var == 0.0 {
            return 0.0;
        }
        let ac = r.windows(2).map(|w| w[0] * w[1]).sum::<f64>() / var;
        if ac.abs() < self.min_abs {
            return 0.0;
        }
        self.side.apply((ac * r[r.len() - 1]).signum())
    }
}

pub fn specs() -> Vec<Spec> {
    let mut v = Vec::new();
    for side in Side::BOTH {
        let (st, sw) = (side.tag(), side.words());
        for (q, tag) in [(1e-6, "slow"), (1e-5, "mid"), (1e-4, "fast")] {
            v.push(Spec::per_symbol(
                format!("statistical.kalman_trend.{tag}.{st}"),
                "statistical",
                format!(
                    "{sw}: sign of the velocity from a local-linear-trend Kalman filter (q={q:e})."
                ),
                move || KalmanTrend {
                    x: [0.0; 2],
                    p: [[0.0; 2]; 2],
                    q,
                    r: 1e-4,
                    n: 0,
                    side,
                },
            ));
        }
        for (n, m) in [(60, 0.05), (120, 0.1)] {
            v.push(Spec::per_symbol(
                format!("statistical.autocorr.{n}.{m}.{st}"),
                "statistical",
                format!("{sw}: bet on continuation/reversal per the {n}-day lag-1 autocorrelation (|rho| > {m})."),
                move || Autocorr { rets: Window::new(n), min_abs: m, prev: None, side },
            ));
        }
    }
    for (n, k, band) in [(120, 5, 0.1), (250, 10, 0.15), (250, 5, 0.05)] {
        v.push(Spec::per_symbol(
            format!("statistical.variance_ratio.{n}.{k}.{band}"),
            "statistical",
            format!("Regime switch on variance ratio VR({k}) over {n} days: follow if > {}, fade if < {}.", 1.0 + band, 1.0 - band),
            move || VarianceRatio { rets: Window::new(n), k, band, prev: None },
        ));
    }
    for (n, band) in [(128, 0.05), (256, 0.05), (256, 0.1)] {
        v.push(Spec::per_symbol(
            format!("statistical.hurst.{n}.{band}"),
            "statistical",
            format!(
                "Regime switch on the rescaled-range Hurst exponent over {n} days (band {band})."
            ),
            move || Hurst {
                rets: Window::new(n),
                band,
                prev: None,
            },
        ));
    }
    v
}
