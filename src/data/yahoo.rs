//! Yahoo Finance chart API (unofficial, free, daily bars). OHLC are rescaled by
//! `adjclose / close` so the whole bar is split- and dividend-adjusted.

use super::{Bar, Series};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, NaiveDate};
use serde_json::Value;
use std::time::Duration;

pub fn fetch(agent: &ureq::Agent, symbol: &str, start: NaiveDate) -> Result<Series> {
    let p1 = start.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
    let p2 = chrono::Utc::now().timestamp() + 86_400;
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?period1={p1}&period2={p2}&interval=1d&events=div%2Csplit",
        symbol.replace('.', "-")
    );
    let mut last_err = anyhow!("no attempt");
    for attempt in 0..4 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(2u64.pow(attempt)));
        }
        match agent
            .get(&url)
            .set("User-Agent", "Mozilla/5.0 (compatible; scrying-bull)")
            .call()
        {
            Ok(resp) => {
                let v: Value = resp.into_json().context("decoding Yahoo response")?;
                return parse(symbol, &v);
            }
            Err(e) => last_err = anyhow!("{symbol}: {e}"),
        }
    }
    Err(last_err)
}

pub fn parse(symbol: &str, v: &Value) -> Result<Series> {
    let r = &v["chart"]["result"][0];
    if r.is_null() {
        return Err(anyhow!("{symbol}: no result ({})", v["chart"]["error"]));
    }
    let ts = r["timestamp"]
        .as_array()
        .ok_or_else(|| anyhow!("{symbol}: no timestamps"))?;
    let gmtoffset = r["meta"]["gmtoffset"].as_i64().unwrap_or(0);
    let q = &r["indicators"]["quote"][0];
    let col = |name: &str| -> Vec<Option<f64>> {
        q[name]
            .as_array()
            .map(|a| a.iter().map(Value::as_f64).collect())
            .unwrap_or_default()
    };
    let (o, h, l, c, vol) = (
        col("open"),
        col("high"),
        col("low"),
        col("close"),
        col("volume"),
    );
    let adj: Vec<Option<f64>> = r["indicators"]["adjclose"][0]["adjclose"]
        .as_array()
        .map(|a| a.iter().map(Value::as_f64).collect())
        .unwrap_or_default();

    let mut bars = Vec::with_capacity(ts.len());
    for (k, t) in ts.iter().enumerate() {
        let Some(t) = t.as_i64() else { continue };
        let get = |v: &Vec<Option<f64>>| v.get(k).copied().flatten();
        let (Some(open), Some(high), Some(low), Some(close)) = (get(&o), get(&h), get(&l), get(&c))
        else {
            continue;
        };
        let factor = get(&adj)
            .filter(|a| *a > 0.0 && close > 0.0)
            .map_or(1.0, |a| a / close);
        let Some(dt) = DateTime::from_timestamp(t + gmtoffset, 0) else {
            continue;
        };
        let bar = Bar {
            date: dt.date_naive(),
            open: open * factor,
            high: high * factor,
            low: low * factor,
            close: close * factor,
            volume: get(&vol).unwrap_or(0.0),
        };
        if bar.is_sane() {
            bars.push(bar);
        }
    }
    bars.sort_by_key(|b| b.date);
    bars.dedup_by_key(|b| b.date);
    Ok(Series {
        symbol: symbol.to_string(),
        bars,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chart_payload_and_adjusts() {
        let v: Value = serde_json::from_str(
            r#"{"chart":{"result":[{"meta":{"gmtoffset":-18000},
            "timestamp":[1704205800,1704292200,1704378600],
            "indicators":{"quote":[{"open":[10,null,12],"high":[11,11,13],"low":[9,9,11],
            "close":[10,10.5,12],"volume":[100,200,300]}],
            "adjclose":[{"adjclose":[5,5.25,12]}]}}],"error":null}}"#,
        )
        .unwrap();
        let s = parse("T", &v).unwrap();
        assert_eq!(s.bars.len(), 2); // null open dropped
        assert_eq!(s.bars[0].date, NaiveDate::from_ymd_opt(2024, 1, 2).unwrap());
        assert!((s.bars[0].open - 5.0).abs() < 1e-12); // halved by adjclose factor
        assert!((s.bars[1].close - 12.0).abs() < 1e-12);
    }
}
