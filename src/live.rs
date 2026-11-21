//! Mirror paper-ledger strategies into a real (or broker-paper) brokerage account.
//!
//! The broker account copies what the chosen strategies' *paper accounts* will
//! hold after the next open, scaled to a fixed dollar budget. Orders are sent
//! after the close as market/day orders, so they fill at the next open: the same
//! timing the backtests and the paper ledger assume.

use crate::broker::{Broker, BrokerPosition, Order, ORDER_PREFIX};
use crate::paper::Ledger;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Alpaca's paper environment (fake money, real order flow).
    Paper,
    /// Real money.
    Live,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveConfig {
    pub mode: Mode,
    /// Strategy ids to mirror; their weights are averaged.
    pub strategies: Vec<String>,
    /// Dollars to deploy. The bot never manages more than this.
    pub budget: f64,
    /// Smallest order to send, in dollars.
    #[serde(default = "default_min_order")]
    pub min_order: f64,
    /// Skip rebalances smaller than this fraction of the budget.
    #[serde(default = "default_band")]
    pub band: f64,
    /// Kill switch: if the managed value falls below this, liquidate and stop trading.
    pub halt_below: f64,
}
fn default_min_order() -> f64 {
    5.0
}
fn default_band() -> f64 {
    0.02
}

/// The bot's own books, persisted next to the paper ledger. The bot tracks its
/// *own* cash (budget, minus buys, plus sells of its own orders), so other money
/// in the brokerage account is never used and the kill switch measures only the
/// bot's performance.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LiveState {
    pub mode: Option<Mode>,
    pub bot_cash: f64,
    /// Total budget credited so far (raising `budget` in config adds the difference).
    pub funded: f64,
    /// RFC 3339 time of the last fill reconciliation.
    pub last_sync: Option<String>,
    /// Broker order ids already reconciled (recent ones only).
    pub seen: Vec<String>,
    /// Set once the kill switch fires. Trading stays off until this is cleared by hand.
    pub halted: Option<String>,
    /// (date, managed value) after each run.
    pub history: Vec<(chrono::NaiveDate, f64)>,
}

impl LiveState {
    pub fn load(path: &std::path::Path) -> Result<LiveState> {
        if !path.exists() {
            return Ok(LiveState::default());
        }
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Credit new budget, then apply fills of our own orders since the last sync.
    pub fn reconcile(
        &mut self,
        cfg: &LiveConfig,
        broker: &dyn Broker,
        now: &str,
    ) -> Result<Vec<String>> {
        let mut log = Vec::new();
        if let Some(m) = self.mode {
            if m != cfg.mode {
                bail!("live state was created in {m:?} mode but config says {:?}; use a separate state file", cfg.mode);
            }
        }
        self.mode = Some(cfg.mode);
        if cfg.budget > self.funded {
            log.push(format!("budget funded +${:.2}", cfg.budget - self.funded));
            self.bot_cash += cfg.budget - self.funded;
            self.funded = cfg.budget;
        }
        let after = self
            .last_sync
            .clone()
            .unwrap_or_else(|| "2000-01-01T00:00:00Z".into());
        // Look back a day past the last sync: orders can fill after we last looked.
        let after = chrono::DateTime::parse_from_rfc3339(&after)
            .map(|t| {
                (t - chrono::Duration::days(3)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            })
            .unwrap_or(after);
        for f in broker.closed_orders(&after)? {
            if !f.client_order_id.starts_with(ORDER_PREFIX) || self.seen.contains(&f.id) {
                continue;
            }
            let amt = f.qty * f.avg_price;
            self.bot_cash += if f.buy { -amt } else { amt };
            log.push(format!(
                "fill: {} {:.6} {} @ {:.4} (${amt:.2})",
                if f.buy { "bought" } else { "sold" },
                f.qty,
                f.symbol,
                f.avg_price
            ));
            self.seen.push(f.id);
        }
        let keep = self.seen.len().saturating_sub(2000);
        self.seen.drain(..keep);
        self.last_sync = Some(now.to_string());
        Ok(log)
    }
}

/// Weights the strategy's paper account will hold after the next open
/// (pending orders applied, everything else at current drifted weights).
/// Shorts are dropped: the live account is long-only.
pub fn paper_weights(ledger: &Ledger, id: &str) -> Result<BTreeMap<String, f64>> {
    let Some(acc) = ledger.accounts.get(id) else {
        bail!("strategy {id} has no paper account yet (run `scry paper` first)");
    };
    if !acc.active {
        bail!("strategy {id} is no longer in the registry");
    }
    let equity = acc.cash
        + acc
            .positions
            .values()
            .map(|p| p.qty * p.last_px)
            .sum::<f64>();
    if equity <= 0.0 {
        bail!("strategy {id} paper equity is {equity}");
    }
    let mut w: BTreeMap<String, f64> = acc
        .positions
        .iter()
        .map(|(s, p)| {
            (
                s.clone(),
                if p.pending {
                    p.target
                } else {
                    p.qty * p.last_px / equity
                }
                .max(0.0),
            )
        })
        .collect();
    let gross: f64 = w.values().sum();
    if gross > 1.0 {
        w.values_mut().for_each(|x| *x /= gross);
    }
    Ok(w)
}

/// Average the target weights of several strategies.
pub fn blend(ledger: &Ledger, ids: &[String]) -> Result<BTreeMap<String, f64>> {
    if ids.is_empty() {
        bail!("live.strategies is empty");
    }
    let mut out: BTreeMap<String, f64> = BTreeMap::new();
    for id in ids {
        for (s, w) in paper_weights(ledger, id)? {
            *out.entry(s).or_default() += w / ids.len() as f64;
        }
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
pub struct Plan {
    /// Dollars the bot is managing this run.
    pub capital: f64,
    /// Current value of managed positions + cash allotted.
    pub managed_value: f64,
    pub halted: bool,
    pub orders: Vec<Order>,
    pub notes: Vec<String>,
}

/// Turn target weights into orders. Pure function: no I/O.
///
/// * `universe`: only these symbols are ever touched; other holdings are ignored.
/// * `bot_cash`: the bot's own uninvested cash; `broker_cash`: what the account
///   actually has. Buys must fit within both (no margin, no borrowing other money).
pub fn plan(
    cfg: &LiveConfig,
    weights: &BTreeMap<String, f64>,
    universe: &[String],
    positions: &[BrokerPosition],
    prices: &BTreeMap<String, f64>,
    bot_cash: f64,
    broker_cash: f64,
) -> Plan {
    let mut notes = Vec::new();
    let held: BTreeMap<&str, &BrokerPosition> = positions
        .iter()
        .filter(|p| universe.contains(&p.symbol))
        .map(|p| (p.symbol.as_str(), p))
        .collect();
    let invested: f64 = held.values().map(|p| p.market_value).sum();
    let managed_value = invested + bot_cash;
    // Gains compound; the bot never sizes beyond what it actually has.
    let capital = managed_value.max(0.0);

    if managed_value < cfg.halt_below {
        notes.push(format!(
            "HALT: managed value ${managed_value:.2} is below halt_below ${:.2}; liquidating",
            cfg.halt_below
        ));
        let orders = held
            .values()
            .map(|p| Order::Close {
                symbol: p.symbol.clone(),
                qty: p.qty,
            })
            .collect();
        return Plan {
            capital,
            managed_value,
            halted: true,
            orders,
            notes,
        };
    }

    let threshold = cfg.min_order.max(cfg.band * capital);
    let (mut sells, mut buys) = (Vec::new(), Vec::new());
    for sym in universe {
        let want = weights.get(sym).copied().unwrap_or(0.0).max(0.0) * capital;
        let pos = held.get(sym.as_str());
        let cur = pos.map_or(0.0, |p| p.market_value);
        let delta = want - cur;
        if want < cfg.min_order && cur > 0.0 {
            sells.push(Order::Close {
                symbol: sym.clone(),
                qty: pos.unwrap().qty,
            });
            continue;
        }
        if delta.abs() < threshold {
            continue;
        }
        if delta < 0.0 {
            let p = pos.unwrap();
            let price = if p.price > 0.0 {
                p.price
            } else {
                prices.get(sym).copied().unwrap_or(0.0)
            };
            if price <= 0.0 {
                notes.push(format!("{sym}: no price, skipping sell"));
                continue;
            }
            let qty = ((-delta / price) * 1e6).floor() / 1e6;
            if qty >= p.qty * 0.999 {
                sells.push(Order::Close {
                    symbol: sym.clone(),
                    qty: p.qty,
                });
            } else if qty > 0.0 {
                sells.push(Order::Sell {
                    symbol: sym.clone(),
                    qty,
                });
            }
        } else {
            buys.push((sym.clone(), delta));
        }
    }

    // Fund buys from cash plus expected sale proceeds; never use margin.
    let proceeds: f64 = sells
        .iter()
        .map(|o| match o {
            Order::Close { symbol, qty } | Order::Sell { symbol, qty } => {
                qty * held.get(symbol.as_str()).map_or(0.0, |p| p.price)
            }
            Order::Buy { .. } => 0.0,
        })
        .sum();
    let available = (bot_cash.min(broker_cash) + proceeds) * 0.98; // headroom for slippage
    let wanted: f64 = buys.iter().map(|b| b.1).sum();
    let scale = if wanted > available && wanted > 0.0 {
        notes.push(format!(
            "buys scaled to {:.0}% to fit available cash ${available:.2}",
            100.0 * available / wanted
        ));
        (available / wanted).max(0.0)
    } else {
        1.0
    };
    let mut orders = sells;
    for (symbol, amt) in buys {
        let notional = (amt * scale * 100.0).floor() / 100.0;
        if notional >= cfg.min_order {
            orders.push(Order::Buy { symbol, notional });
        }
    }
    Plan {
        capital,
        managed_value,
        halted: false,
        orders,
        notes,
    }
}

#[derive(Debug, Serialize)]
pub struct Fill {
    pub order: Order,
    pub result: String,
}

/// Cancel our stale open orders, then submit the plan. Returns per-order outcomes.
pub fn execute(broker: &dyn Broker, plan: &Plan, run_tag: &str) -> Result<Vec<Fill>> {
    for o in broker.open_orders()? {
        if o.client_order_id.starts_with(ORDER_PREFIX) {
            broker.cancel(&o.id)?;
            println!("cancelled stale order {} ({})", o.client_order_id, o.symbol);
        }
    }
    let mut out = Vec::new();
    for (k, order) in plan.orders.iter().enumerate() {
        let coid = format!("{ORDER_PREFIX}{run_tag}-{k}");
        let result = match broker.submit(order, &coid) {
            Ok(id) => format!("submitted {id}"),
            Err(e) => format!("error: {e:#}"),
        };
        println!("{:?} -> {result}", order);
        out.push(Fill {
            order: order.clone(),
            result,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> LiveConfig {
        LiveConfig {
            mode: Mode::Paper,
            strategies: vec![],
            budget: 1000.0,
            min_order: 5.0,
            band: 0.02,
            halt_below: 500.0,
        }
    }
    fn pos(s: &str, qty: f64, px: f64) -> BrokerPosition {
        BrokerPosition {
            symbol: s.into(),
            qty,
            market_value: qty * px,
            price: px,
        }
    }
    fn uni() -> Vec<String> {
        vec!["AAA".into(), "BBB".into(), "CCC".into()]
    }
    fn buys(p: &Plan) -> f64 {
        p.orders
            .iter()
            .map(|o| {
                if let Order::Buy { notional, .. } = o {
                    *notional
                } else {
                    0.0
                }
            })
            .sum()
    }

    #[test]
    fn fresh_account_buys_targets() {
        let w = BTreeMap::from([("AAA".to_string(), 0.5), ("BBB".to_string(), 0.25)]);
        let p = plan(&cfg(), &w, &uni(), &[], &BTreeMap::new(), 1000.0, 5000.0);
        assert_eq!(p.capital, 1000.0);
        assert_eq!(
            p.orders,
            vec![
                Order::Buy {
                    symbol: "AAA".into(),
                    notional: 500.0
                },
                Order::Buy {
                    symbol: "BBB".into(),
                    notional: 250.0
                }
            ]
        );
    }

    #[test]
    fn ignores_holdings_outside_universe_and_other_cash() {
        let w = BTreeMap::from([("AAA".to_string(), 1.0)]);
        let p = plan(
            &cfg(),
            &w,
            &uni(),
            &[pos("ZZZ", 100.0, 50.0)],
            &BTreeMap::new(),
            1000.0,
            50_000.0,
        );
        assert_eq!(p.orders.len(), 1);
        assert!(buys(&p) <= 1000.0);
    }

    #[test]
    fn sells_before_buys_and_closes_exits() {
        let w = BTreeMap::from([("BBB".to_string(), 0.9)]);
        let p = plan(
            &cfg(),
            &w,
            &uni(),
            &[pos("AAA", 10.0, 90.0)],
            &BTreeMap::new(),
            100.0,
            100.0,
        );
        assert_eq!(
            p.orders[0],
            Order::Close {
                symbol: "AAA".into(),
                qty: 10.0
            }
        );
        match &p.orders[1] {
            Order::Buy { symbol, notional } => {
                assert!(symbol == "BBB" && *notional > 850.0 && *notional <= 900.0)
            }
            o => panic!("unexpected {o:?}"),
        }
    }

    #[test]
    fn small_changes_are_skipped() {
        let w = BTreeMap::from([("AAA".to_string(), 0.51)]);
        let p = plan(
            &cfg(),
            &w,
            &uni(),
            &[pos("AAA", 5.0, 100.0)],
            &BTreeMap::new(),
            500.0,
            500.0,
        );
        assert!(p.orders.is_empty(), "{:?}", p.orders);
    }

    #[test]
    fn partial_sell_uses_fractional_qty() {
        let w = BTreeMap::from([("AAA".to_string(), 0.25)]);
        let p = plan(
            &cfg(),
            &w,
            &uni(),
            &[pos("AAA", 10.0, 100.0)],
            &BTreeMap::new(),
            0.0,
            0.0,
        );
        assert_eq!(
            p.orders,
            vec![Order::Sell {
                symbol: "AAA".into(),
                qty: 7.5
            }]
        );
    }

    /// Losses can't be masked by other cash in the account.
    #[test]
    fn halts_on_bot_losses_even_with_rich_account() {
        let w = BTreeMap::from([("AAA".to_string(), 1.0)]);
        let p = plan(
            &cfg(),
            &w,
            &uni(),
            &[pos("AAA", 4.0, 100.0)],
            &BTreeMap::new(),
            0.0,
            100_000.0,
        );
        assert!(p.halted);
        assert_eq!(
            p.orders,
            vec![Order::Close {
                symbol: "AAA".into(),
                qty: 4.0
            }]
        );
    }

    #[test]
    fn no_margin() {
        let w = BTreeMap::from([("AAA".to_string(), 1.0)]);
        let p = plan(&cfg(), &w, &uni(), &[], &BTreeMap::new(), 1000.0, 600.0);
        assert!(buys(&p) <= 600.0);
    }

    use crate::broker::{AccountInfo, FilledOrder, OpenOrder};
    use std::cell::RefCell;

    #[derive(Default)]
    struct Mock {
        fills: Vec<FilledOrder>,
        open: Vec<OpenOrder>,
        cancelled: RefCell<Vec<String>>,
        submitted: RefCell<Vec<(Order, String)>>,
    }
    impl Broker for Mock {
        fn account(&self) -> Result<AccountInfo> {
            Ok(AccountInfo {
                equity: 1e4,
                cash: 1e4,
                buying_power: 1e4,
                blocked: false,
            })
        }
        fn positions(&self) -> Result<Vec<BrokerPosition>> {
            Ok(vec![])
        }
        fn market_open(&self) -> Result<bool> {
            Ok(false)
        }
        fn open_orders(&self) -> Result<Vec<OpenOrder>> {
            Ok(self.open.clone())
        }
        fn cancel(&self, id: &str) -> Result<()> {
            self.cancelled.borrow_mut().push(id.into());
            Ok(())
        }
        fn closed_orders(&self, _: &str) -> Result<Vec<FilledOrder>> {
            Ok(self.fills.clone())
        }
        fn submit(&self, o: &Order, coid: &str) -> Result<String> {
            self.submitted.borrow_mut().push((o.clone(), coid.into()));
            Ok("x".into())
        }
    }
    fn fill(id: &str, coid: &str, buy: bool, qty: f64, px: f64) -> FilledOrder {
        FilledOrder {
            id: id.into(),
            client_order_id: coid.into(),
            symbol: "AAA".into(),
            buy,
            qty,
            avg_price: px,
        }
    }

    #[test]
    fn reconcile_counts_only_our_fills_once() {
        let m = Mock {
            fills: vec![
                fill("1", "scry-a-0", true, 2.0, 100.0),
                fill("2", "manual", true, 50.0, 10.0),
                fill("3", "scry-a-1", false, 1.0, 110.0),
            ],
            ..Default::default()
        };
        let mut st = LiveState::default();
        st.reconcile(&cfg(), &m, "2026-01-01T00:00:00Z").unwrap();
        assert!((st.bot_cash - (1000.0 - 200.0 + 110.0)).abs() < 1e-9);
        st.reconcile(&cfg(), &m, "2026-01-02T00:00:00Z").unwrap(); // same fills again
        assert!((st.bot_cash - 910.0).abs() < 1e-9);
        // Raising the budget funds the difference once.
        let mut c = cfg();
        c.budget = 1500.0;
        st.reconcile(&c, &m, "2026-01-03T00:00:00Z").unwrap();
        assert!((st.bot_cash - 1410.0).abs() < 1e-9);
        // Switching paper <-> live on the same state is refused.
        c.mode = Mode::Live;
        assert!(st.reconcile(&c, &m, "2026-01-04T00:00:00Z").is_err());
    }

    #[test]
    fn execute_cancels_only_our_orders() {
        let m = Mock {
            open: vec![
                OpenOrder {
                    id: "o1".into(),
                    client_order_id: "scry-old-0".into(),
                    symbol: "AAA".into(),
                },
                OpenOrder {
                    id: "o2".into(),
                    client_order_id: "my-manual-order".into(),
                    symbol: "BBB".into(),
                },
            ],
            ..Default::default()
        };
        let w = BTreeMap::from([("AAA".to_string(), 1.0)]);
        let p = plan(&cfg(), &w, &uni(), &[], &BTreeMap::new(), 1000.0, 1000.0);
        execute(&m, &p, "t").unwrap();
        assert_eq!(*m.cancelled.borrow(), vec!["o1".to_string()]);
        let sub = m.submitted.borrow();
        assert_eq!(sub.len(), 1);
        assert!(sub[0].1.starts_with(ORDER_PREFIX));
    }
}
