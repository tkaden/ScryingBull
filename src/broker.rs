//! Broker connectivity. One implementation today: Alpaca (paper or live).
//!
//! Credentials come only from the environment (`ALPACA_KEY_ID`,
//! `ALPACA_SECRET_KEY`), never from config files.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

/// Prefix on every client order id we create, so we only ever cancel our own orders.
pub const ORDER_PREFIX: &str = "scry-";

#[derive(Clone, Debug)]
pub struct AccountInfo {
    pub equity: f64,
    pub cash: f64,
    pub buying_power: f64,
    pub blocked: bool,
}

#[derive(Clone, Debug)]
pub struct BrokerPosition {
    pub symbol: String,
    pub qty: f64,
    pub market_value: f64,
    pub price: f64,
}

#[derive(Clone, Debug)]
pub struct OpenOrder {
    pub id: String,
    pub client_order_id: String,
    pub symbol: String,
}

/// A (partially) filled order, used to reconcile the bot's own cash.
#[derive(Clone, Debug)]
pub struct FilledOrder {
    pub id: String,
    pub client_order_id: String,
    pub symbol: String,
    pub buy: bool,
    pub qty: f64,
    pub avg_price: f64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Order {
    /// Buy a dollar amount (fractional shares).
    Buy { symbol: String, notional: f64 },
    /// Sell a (possibly fractional) share quantity.
    Sell { symbol: String, qty: f64 },
    /// Liquidate the whole position (`qty` = full holding).
    Close { symbol: String, qty: f64 },
}

impl Order {
    pub fn symbol(&self) -> &str {
        match self {
            Order::Buy { symbol, .. }
            | Order::Sell { symbol, .. }
            | Order::Close { symbol, .. } => symbol,
        }
    }
}

pub trait Broker {
    fn account(&self) -> Result<AccountInfo>;
    fn positions(&self) -> Result<Vec<BrokerPosition>>;
    fn market_open(&self) -> Result<bool>;
    fn open_orders(&self) -> Result<Vec<OpenOrder>>;
    fn cancel(&self, id: &str) -> Result<()>;
    /// Orders closed (filled, partially filled then cancelled, ...) since `after` (RFC 3339).
    fn closed_orders(&self, after: &str) -> Result<Vec<FilledOrder>>;
    /// Submit an order; returns the broker's order id.
    fn submit(&self, order: &Order, client_order_id: &str) -> Result<String>;
}

pub struct Alpaca {
    agent: ureq::Agent,
    base: String,
    key: String,
    secret: String,
}

/// Our tickers use Yahoo style (`BRK-B`); Alpaca uses `BRK.B`.
pub fn to_alpaca(sym: &str) -> String {
    sym.replace('-', ".")
}
pub fn from_alpaca(sym: &str) -> String {
    sym.replace('.', "-")
}

fn num(v: &Value) -> f64 {
    match v {
        Value::String(s) => s.parse().unwrap_or(0.0),
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    }
}

impl Alpaca {
    pub fn from_env(live: bool) -> Result<Alpaca> {
        let key = std::env::var("ALPACA_KEY_ID").context("ALPACA_KEY_ID is not set")?;
        let secret = std::env::var("ALPACA_SECRET_KEY").context("ALPACA_SECRET_KEY is not set")?;
        let base = if live {
            "https://api.alpaca.markets"
        } else {
            "https://paper-api.alpaca.markets"
        };
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(30))
            .try_proxy_from_env(true)
            .build();
        Ok(Alpaca {
            agent,
            base: base.into(),
            key,
            secret,
        })
    }

    fn req(&self, method: &str, path: &str) -> ureq::Request {
        self.agent
            .request(method, &format!("{}{path}", self.base))
            .set("APCA-API-KEY-ID", &self.key)
            .set("APCA-API-SECRET-KEY", &self.secret)
    }

    fn call(&self, r: ureq::Request, body: Option<Value>) -> Result<Value> {
        let resp = match body {
            Some(b) => r.send_json(b),
            None => r.call(),
        };
        match resp {
            Ok(resp) => {
                let text = resp.into_string()?;
                Ok(if text.trim().is_empty() {
                    Value::Null
                } else {
                    serde_json::from_str(&text)?
                })
            }
            Err(ureq::Error::Status(code, resp)) => {
                let text = resp.into_string().unwrap_or_default();
                Err(anyhow!("Alpaca HTTP {code}: {text}"))
            }
            Err(e) => Err(anyhow!("Alpaca request failed: {e}")),
        }
    }
}

#[derive(Deserialize)]
struct RawOrder {
    id: String,
    client_order_id: String,
    symbol: String,
}

impl Broker for Alpaca {
    fn account(&self) -> Result<AccountInfo> {
        let v = self.call(self.req("GET", "/v2/account"), None)?;
        Ok(AccountInfo {
            equity: num(&v["equity"]),
            cash: num(&v["cash"]),
            buying_power: num(&v["buying_power"]),
            blocked: v["trading_blocked"].as_bool().unwrap_or(false)
                || v["account_blocked"].as_bool().unwrap_or(false),
        })
    }

    fn positions(&self) -> Result<Vec<BrokerPosition>> {
        let v = self.call(self.req("GET", "/v2/positions"), None)?;
        let arr = v
            .as_array()
            .ok_or_else(|| anyhow!("unexpected positions payload"))?;
        Ok(arr
            .iter()
            .map(|p| BrokerPosition {
                symbol: from_alpaca(p["symbol"].as_str().unwrap_or_default()),
                qty: num(&p["qty"]),
                market_value: num(&p["market_value"]),
                price: num(&p["current_price"]),
            })
            .collect())
    }

    fn market_open(&self) -> Result<bool> {
        Ok(self.call(self.req("GET", "/v2/clock"), None)?["is_open"]
            .as_bool()
            .unwrap_or(false))
    }

    fn open_orders(&self) -> Result<Vec<OpenOrder>> {
        let v = self.call(self.req("GET", "/v2/orders?status=open&limit=500"), None)?;
        let raw: Vec<RawOrder> = serde_json::from_value(v)?;
        Ok(raw
            .into_iter()
            .map(|o| OpenOrder {
                id: o.id,
                client_order_id: o.client_order_id,
                symbol: from_alpaca(&o.symbol),
            })
            .collect())
    }

    fn cancel(&self, id: &str) -> Result<()> {
        self.call(self.req("DELETE", &format!("/v2/orders/{id}")), None)
            .map(|_| ())
    }

    fn closed_orders(&self, after: &str) -> Result<Vec<FilledOrder>> {
        let path = format!("/v2/orders?status=closed&limit=500&direction=asc&after={after}");
        let v = self.call(self.req("GET", &path), None)?;
        let arr = v
            .as_array()
            .ok_or_else(|| anyhow!("unexpected orders payload"))?;
        Ok(arr
            .iter()
            .filter(|o| num(&o["filled_qty"]) > 0.0)
            .map(|o| FilledOrder {
                id: o["id"].as_str().unwrap_or_default().into(),
                client_order_id: o["client_order_id"].as_str().unwrap_or_default().into(),
                symbol: from_alpaca(o["symbol"].as_str().unwrap_or_default()),
                buy: o["side"].as_str() == Some("buy"),
                qty: num(&o["filled_qty"]),
                avg_price: num(&o["filled_avg_price"]),
            })
            .collect())
    }

    fn submit(&self, order: &Order, client_order_id: &str) -> Result<String> {
        let v = match order {
            Order::Buy { symbol, notional } => self.call(
                self.req("POST", "/v2/orders"),
                Some(json!({
                    "symbol": to_alpaca(symbol), "notional": format!("{notional:.2}"), "side": "buy",
                    "type": "market", "time_in_force": "day", "client_order_id": client_order_id,
                })),
            )?,
            Order::Sell { symbol, qty } | Order::Close { symbol, qty } => {
                if *qty <= 0.0 {
                    bail!("refusing to sell non-positive qty");
                }
                self.call(
                    self.req("POST", "/v2/orders"),
                    Some(json!({
                        "symbol": to_alpaca(symbol), "qty": format!("{qty:.6}"), "side": "sell",
                        "type": "market", "time_in_force": "day", "client_order_id": client_order_id,
                    })),
                )?
            }
        };
        Ok(v["id"].as_str().unwrap_or("").to_string())
    }
}
