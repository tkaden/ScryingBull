//! Company financials from SEC EDGAR's XBRL "companyfacts" API.
//!
//! Every value keeps the date it was *filed*, so a backtest at date `t` only sees
//! numbers that were public by `t` (point-in-time, no lookahead). Flows (income,
//! cash flow) come from annual 10-K figures; balance-sheet items and share counts
//! use the latest 10-K or 10-Q.
//!
//! SEC requires automated clients to identify themselves with a contact email in
//! the User-Agent. Set it in the `SEC_USER_AGENT` environment variable, or in a
//! `.sec_user_agent` file in the working directory (git-ignored), e.g.
//! `ScryingBull research you@example.com`.

use anyhow::{anyhow, bail, Context, Result};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// One reported number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub end: NaiveDate,
    pub val: f64,
    pub filed: NaiveDate,
}

/// Compact per-company store of the fields we use.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Facts {
    pub cik: u64,
    pub name: String,
    pub fetched: Option<NaiveDate>,
    pub fields: BTreeMap<String, Vec<Fact>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// 12-month duration from annual reports.
    Annual,
    /// Point-in-time balance from any 10-K/10-Q.
    Instant,
}

/// (our field, kind, taxonomy, candidate concepts in priority order)
const FIELDS: &[(&str, Kind, &str, &[&str])] = &[
    (
        "net_income",
        Kind::Annual,
        "us-gaap",
        &[
            "NetIncomeLoss",
            "ProfitLoss",
            "NetIncomeLossAvailableToCommonStockholdersBasic",
        ],
    ),
    (
        "revenue",
        Kind::Annual,
        "us-gaap",
        &[
            "Revenues",
            "RevenueFromContractWithCustomerExcludingAssessedTax",
            "SalesRevenueNet",
            "RevenueFromContractWithCustomerIncludingAssessedTax",
        ],
    ),
    (
        "cfo",
        Kind::Annual,
        "us-gaap",
        &[
            "NetCashProvidedByUsedInOperatingActivities",
            "NetCashProvidedByUsedInOperatingActivitiesContinuingOperations",
        ],
    ),
    (
        "capex",
        Kind::Annual,
        "us-gaap",
        &[
            "PaymentsToAcquirePropertyPlantAndEquipment",
            "PaymentsToAcquireProductiveAssets",
        ],
    ),
    (
        "op_income",
        Kind::Annual,
        "us-gaap",
        &["OperatingIncomeLoss"],
    ),
    ("gross_profit", Kind::Annual, "us-gaap", &["GrossProfit"]),
    (
        "diluted_shares",
        Kind::Annual,
        "us-gaap",
        &["WeightedAverageNumberOfDilutedSharesOutstanding"],
    ),
    (
        "equity",
        Kind::Instant,
        "us-gaap",
        &[
            "StockholdersEquity",
            "StockholdersEquityIncludingPortionAttributableToNoncontrollingInterest",
        ],
    ),
    ("assets", Kind::Instant, "us-gaap", &["Assets"]),
    (
        "assets_current",
        Kind::Instant,
        "us-gaap",
        &["AssetsCurrent"],
    ),
    (
        "liabilities_current",
        Kind::Instant,
        "us-gaap",
        &["LiabilitiesCurrent"],
    ),
    (
        "lt_debt",
        Kind::Instant,
        "us-gaap",
        &["LongTermDebtNoncurrent", "LongTermDebt"],
    ),
    (
        "cash",
        Kind::Instant,
        "us-gaap",
        &[
            "CashAndCashEquivalentsAtCarryingValue",
            "CashCashEquivalentsRestrictedCashAndRestrictedCashEquivalents",
        ],
    ),
    (
        "shares",
        Kind::Instant,
        "dei",
        &["EntityCommonStockSharesOutstanding"],
    ),
];

fn date(v: &Value) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(v.as_str()?, "%Y-%m-%d").ok()
}

/// Extract our fields from a companyfacts JSON document.
pub fn parse_companyfacts(v: &Value) -> Result<Facts> {
    let facts = &v["facts"];
    if facts.is_null() {
        bail!("no facts in response");
    }
    let mut out = Facts {
        cik: v["cik"].as_u64().unwrap_or(0),
        name: v["entityName"].as_str().unwrap_or_default().to_string(),
        ..Default::default()
    };
    for (field, kind, tax, concepts) in FIELDS {
        let mut merged: BTreeMap<(NaiveDate, NaiveDate), f64> = BTreeMap::new(); // (end, filed) -> val
        for concept in *concepts {
            let units = &facts[*tax][*concept]["units"];
            let Some(units) = units.as_object() else {
                continue;
            };
            for (unit, list) in units {
                if !(unit == "USD" || unit == "shares") {
                    continue;
                }
                for f in list.as_array().into_iter().flatten() {
                    let form = f["form"].as_str().unwrap_or("");
                    let (Some(end), Some(filed), Some(val)) =
                        (date(&f["end"]), date(&f["filed"]), f["val"].as_f64())
                    else {
                        continue;
                    };
                    let ok = match kind {
                        Kind::Annual => {
                            let annual_form = form.starts_with("10-K");
                            let days = date(&f["start"]).map(|s| (end - s).num_days()).unwrap_or(0);
                            annual_form && (350..=380).contains(&days)
                        }
                        Kind::Instant => form.starts_with("10-K") || form.starts_with("10-Q"),
                    };
                    // Earlier concepts in the list win when two report the same period.
                    if ok {
                        merged.entry((end, filed)).or_insert(val);
                    }
                }
            }
        }
        // Keep the first filing of each (end, value): later re-reports of an unchanged
        // number add nothing, while restated values are kept with their own date.
        let mut facts: Vec<Fact> = Vec::new();
        for ((end, filed), val) in merged {
            if !facts.iter().any(|f| f.end == end && f.val == val) {
                facts.push(Fact { end, val, filed });
            }
        }
        facts.sort_by_key(|f| (f.filed, f.end));
        if !facts.is_empty() {
            out.fields.insert(field.to_string(), facts);
        }
    }
    Ok(out)
}

impl Facts {
    /// The latest-period value known at `t` (filed on or before `t`), using the most
    /// recent filing for that period. `max_age_days` bounds how stale the period may be.
    pub fn as_of(&self, field: &str, t: NaiveDate, max_age_days: i64) -> Option<&Fact> {
        let list = self.fields.get(field)?;
        let known = list.iter().filter(|f| f.filed <= t);
        let latest_end = known.clone().map(|f| f.end).max()?;
        if (t - latest_end).num_days() > max_age_days {
            return None;
        }
        known
            .filter(|f| f.end == latest_end)
            .max_by_key(|f| f.filed)
    }

    /// The value for the period about a year before `cur` (for year-over-year changes).
    pub fn prior_year(&self, field: &str, cur: &Fact, t: NaiveDate) -> Option<&Fact> {
        let list = self.fields.get(field)?;
        list.iter()
            .filter(|f| f.filed <= t && (300..=430).contains(&(cur.end - f.end).num_days()))
            .max_by_key(|f| (f.end, f.filed))
    }
}

pub struct Sec {
    agent: ureq::Agent,
    ua: String,
    dir: PathBuf,
}

/// The SEC User-Agent from `SEC_USER_AGENT` or `.sec_user_agent`.
pub fn user_agent() -> Result<String> {
    if let Ok(v) = std::env::var("SEC_USER_AGENT") {
        if !v.trim().is_empty() {
            return Ok(v.trim().to_string());
        }
    }
    if let Ok(v) = std::fs::read_to_string(".sec_user_agent") {
        if !v.trim().is_empty() {
            return Ok(v.trim().to_string());
        }
    }
    bail!(
        "SEC EDGAR needs a contact in the User-Agent. Set SEC_USER_AGENT (or write it to .sec_user_agent), \
         e.g. \"ScryingBull research you@example.com\""
    )
}

impl Sec {
    pub fn new(data_dir: &Path) -> Result<Sec> {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(60))
            .try_proxy_from_env(true)
            .build();
        let dir = data_dir.join("sec");
        std::fs::create_dir_all(dir.join("facts"))?;
        Ok(Sec {
            agent,
            ua: user_agent()?,
            dir,
        })
    }

    fn get(&self, url: &str) -> Result<Value> {
        let mut last = anyhow!("no attempt");
        for attempt in 0..4 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs(2u64.pow(attempt)));
            }
            // SEC allows at most 10 requests per second.
            std::thread::sleep(Duration::from_millis(120));
            match self
                .agent
                .get(url)
                .set("User-Agent", &self.ua)
                .set("Accept-Encoding", "gzip")
                .call()
            {
                Ok(r) => return r.into_json().context("decoding SEC response"),
                Err(ureq::Error::Status(404, _)) => bail!("not found: {url}"),
                Err(e) => last = anyhow!("{url}: {e}"),
            }
        }
        Err(last)
    }

    /// Ticker -> CIK map, cached for 30 days.
    pub fn tickers(&self) -> Result<BTreeMap<String, u64>> {
        let path = self.dir.join("company_tickers.json");
        let fresh = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().is_ok_and(|e| e.as_secs() < 30 * 86_400))
            .unwrap_or(false);
        let v: Value = if fresh {
            serde_json::from_str(&std::fs::read_to_string(&path)?)?
        } else {
            let v = self.get("https://www.sec.gov/files/company_tickers.json")?;
            std::fs::write(&path, serde_json::to_string(&v)?)?;
            v
        };
        let obj = v
            .as_object()
            .ok_or_else(|| anyhow!("unexpected ticker file"))?;
        Ok(obj
            .values()
            .filter_map(|e| {
                Some((
                    e["ticker"].as_str()?.to_uppercase().replace('.', "-"),
                    e["cik_str"].as_u64()?,
                ))
            })
            .collect())
    }

    /// Fetch (or reuse a cached copy younger than `max_age_days` of) one company's facts.
    pub fn facts(&self, ticker: &str, cik: u64, max_age_days: i64) -> Result<Facts> {
        let path = self.dir.join("facts").join(format!("{ticker}.json"));
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(f) = serde_json::from_str::<Facts>(&text) {
                if f.fetched
                    .is_some_and(|d| (Utc::now().date_naive() - d).num_days() < max_age_days)
                {
                    return Ok(f);
                }
            }
        }
        let v = self.get(&format!(
            "https://data.sec.gov/api/xbrl/companyfacts/CIK{cik:010}.json"
        ))?;
        let mut f = parse_companyfacts(&v)?;
        f.fetched = Some(Utc::now().date_naive());
        std::fs::write(&path, serde_json::to_string(&f)?)?;
        Ok(f)
    }
}

/// Load cached facts without touching the network.
pub fn load_cached(data_dir: &Path, ticker: &str) -> Option<Facts> {
    let text =
        std::fs::read_to_string(data_dir.join("sec/facts").join(format!("{ticker}.json"))).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_respects_filing_dates() {
        let v: Value = serde_json::from_str(
            r#"{"cik": 1, "entityName": "X", "facts": {"us-gaap": {
              "NetIncomeLoss": {"units": {"USD": [
                {"start":"2019-01-01","end":"2019-12-31","val":10,"form":"10-K","filed":"2020-02-15"},
                {"start":"2019-10-01","end":"2019-12-31","val":3,"form":"10-K","filed":"2020-02-15"},
                {"start":"2019-01-01","end":"2019-12-31","val":10,"form":"10-K","filed":"2021-02-15"},
                {"start":"2020-01-01","end":"2020-12-31","val":12,"form":"10-K","filed":"2021-02-15"}]}},
              "Assets": {"units": {"USD": [
                {"end":"2020-09-30","val":100,"form":"10-Q","filed":"2020-11-01"}]}}},
              "dei": {"EntityCommonStockSharesOutstanding": {"units": {"shares": [
                {"end":"2020-10-20","val":50,"form":"10-Q","filed":"2020-11-01"}]}}}}}"#,
        )
        .unwrap();
        let f = parse_companyfacts(&v).unwrap();
        let ni = &f.fields["net_income"];
        assert_eq!(
            ni.len(),
            2,
            "quarterly value and duplicate re-report dropped: {ni:?}"
        );
        let d = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        assert_eq!(
            f.as_of("net_income", d("2020-03-01"), 400).unwrap().val,
            10.0
        );
        assert!(
            f.as_of("net_income", d("2020-01-15"), 400).is_none(),
            "not yet filed"
        );
        let cur = f.as_of("net_income", d("2021-03-01"), 400).unwrap();
        assert_eq!(cur.val, 12.0);
        assert_eq!(
            f.prior_year("net_income", cur, d("2021-03-01"))
                .unwrap()
                .val,
            10.0
        );
        assert_eq!(f.as_of("shares", d("2020-12-01"), 400).unwrap().val, 50.0);
    }
}
