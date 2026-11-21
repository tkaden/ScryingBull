//! Meta-strategies built from other strategies in the registry.

use crate::data::Bar;
use crate::strategy::{Spec, Strategy};
use chrono::NaiveDate;

/// A member strategy plus the weights it currently wants.
struct Member {
    s: Box<dyn Strategy>,
    w: Vec<f64>,
    buf: Vec<f64>,
    score: f64,
}

impl Member {
    fn step(&mut self, date: NaiveDate, bars: &[Option<Bar>]) {
        if self.w.len() != bars.len() {
            self.w.resize(bars.len(), 0.0);
            self.buf.resize(bars.len(), f64::NAN);
        }
        self.buf.fill(f64::NAN);
        self.s.next(date, bars, &mut self.buf);
        for (w, b) in self.w.iter_mut().zip(&self.buf) {
            if b.is_finite() {
                *w = *b;
            }
        }
    }
}

fn members(ids: &[&str], base: &[Spec]) -> Vec<Member> {
    ids.iter()
        .map(|id| {
            let spec = base
                .iter()
                .find(|s| s.id == *id)
                .unwrap_or_else(|| panic!("ensemble member {id} not in registry"));
            Member {
                s: (spec.build)(),
                w: Vec::new(),
                buf: Vec::new(),
                score: 0.0,
            }
        })
        .collect()
}

/// Average of the members' target weights.
struct Vote {
    m: Vec<Member>,
    last: Vec<f64>,
}
impl Strategy for Vote {
    fn next(&mut self, date: NaiveDate, bars: &[Option<Bar>], w: &mut [f64]) {
        for m in &mut self.m {
            m.step(date, bars);
        }
        self.last.resize(bars.len(), 0.0);
        let k = self.m.len() as f64;
        for i in 0..bars.len() {
            let avg = self.m.iter().map(|m| m.w[i]).sum::<f64>() / k;
            if (avg - self.last[i]).abs() > 1e-12 {
                w[i] = avg;
                self.last[i] = avg;
            }
        }
    }
}

/// Follow the leader: track each member's (paper) daily return with exponential
/// decay and copy whichever member is currently performing best.
struct FollowLeader {
    m: Vec<Member>,
    decay: f64,
    prev_close: Vec<Option<f64>>,
    leader: Option<usize>,
}
impl Strategy for FollowLeader {
    fn next(&mut self, date: NaiveDate, bars: &[Option<Bar>], w: &mut [f64]) {
        self.prev_close.resize(bars.len(), None);
        // Score yesterday's weights against today's close-to-close returns.
        let rets: Vec<f64> = bars
            .iter()
            .zip(&self.prev_close)
            .map(|(b, p)| match (b, p) {
                (Some(b), Some(p)) => b.close / p - 1.0,
                _ => 0.0,
            })
            .collect();
        for m in &mut self.m {
            let r: f64 = m.w.iter().zip(&rets).map(|(w, r)| w * r).sum();
            m.score = m.score * self.decay + r;
            m.step(date, bars);
        }
        for (p, b) in self.prev_close.iter_mut().zip(bars) {
            if let Some(b) = b {
                *p = Some(b.close);
            }
        }
        let best = (0..self.m.len())
            .max_by(|&a, &b| self.m[a].score.total_cmp(&self.m[b].score))
            .unwrap_or(0);
        let switched = self.leader.replace(best) != Some(best);
        // Re-target fully on a switch; otherwise mirror the leader's own changes.
        let lw = &self.m[best];
        for i in 0..w.len() {
            if switched || lw.buf[i].is_finite() {
                w[i] = lw.w[i];
            }
        }
    }
}

pub fn specs(base: &[Spec]) -> Vec<Spec> {
    let groups: [(&str, &str, &[&str]); 4] = [
        (
            "trend_vote",
            "Average of five classic trend signals (MA cross, MACD, Donchian, TSMOM, Supertrend).",
            &[
                "trend.ma_cross.sma.50x200.lo",
                "trend.macd.12.26.9.lo",
                "trend.donchian.55.20.lo",
                "momentum.tsmom.126.lo",
                "trend.supertrend.10.3.lo",
            ],
        ),
        (
            "reversion_vote",
            "Average of four short-term mean-reversion signals.",
            &[
                "mean_reversion.rsi.2.10.70.sma200",
                "mean_reversion.bollinger.20.2.lo",
                "mean_reversion.ibs.0.2.0.7",
                "mean_reversion.down_streak.3",
            ],
        ),
        (
            "family_vote",
            "One representative from each method family, averaged.",
            &[
                "trend.ma_cross.ema.20x100.lo",
                "momentum.tsmom_volscaled.126.lo",
                "mean_reversion.rsi.2.10.70.sma200",
                "volatility.vol_target.20.0.15",
                "volume.obv.50.lo",
                "statistical.kalman_trend.mid.lo",
                "learning.logistic.0.01.0.02.lo",
                "calendar.turn_of_month.1.3",
                "cross_sectional.dual_momentum.126.0.top5",
            ],
        ),
        (
            "learners_vote",
            "Average of the online learners.",
            &[
                "learning.logistic.0.01.0.lo",
                "learning.perceptron.lo",
                "learning.knn.5.30.lo",
                "learning.markov.3.lo",
            ],
        ),
    ];
    let mut v = Vec::new();
    for (name, desc, ids) in groups {
        let base = base.to_vec();
        let ids: Vec<&'static str> = ids.to_vec();
        members(&ids, &base); // validate eagerly
        v.push(Spec::portfolio(
            format!("ensemble.{name}"),
            "ensemble",
            desc,
            move || Vote {
                m: members(&ids, &base),
                last: Vec::new(),
            },
        ));
    }
    let leaders: &[&str] = &[
        "baseline.buy_hold",
        "trend.ma_cross.sma.50x200.lo",
        "mean_reversion.rsi.2.10.70.sma200",
        "momentum.tsmom_volscaled.126.lo",
        "cross_sectional.momentum.126.0.top5",
        "cross_sectional.low_vol.60.top5",
        "learning.logistic.0.01.0.02.lo",
    ];
    for (hl, tag) in [(21.0, "1m"), (63.0, "3m"), (252.0, "1y")] {
        let base = base.to_vec();
        let ids = leaders.to_vec();
        let decay = 0.5f64.powf(1.0 / hl);
        v.push(Spec::portfolio(
            format!("ensemble.follow_leader.{tag}"),
            "ensemble",
            format!("Copy whichever of 7 diverse strategies has the best recent return ({tag} half-life)."),
            move || FollowLeader { m: members(&ids, &base), decay, prev_close: Vec::new(), leader: None },
        ));
    }
    v
}
