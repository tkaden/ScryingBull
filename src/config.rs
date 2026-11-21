//! `scry.toml` configuration.

use crate::engine::Costs;
use crate::live::LiveConfig;
use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Directory holding the per-symbol CSV cache.
    pub data_dir: PathBuf,
    /// First date of history to download / load (warm-up starts here).
    pub data_start: NaiveDate,
    /// Backtests start trading here. [eval_start, validation_start) is the
    /// *train* period: use it to choose strategies and parameters.
    pub eval_start: NaiveDate,
    /// [validation_start, today] is the held-out *validation* period. Judge
    /// choices made on the train period here; never choose based on it.
    pub validation_start: NaiveDate,
    /// Starting capital for every backtest and paper account.
    pub cash: f64,
    pub costs: Costs,
    pub universe: Vec<String>,
    /// Optional second universe for robustness checks in `scry analyze`
    /// (e.g. ETFs only, which has no hindsight stock picking).
    #[serde(default)]
    pub robustness_universe: Vec<String>,
    /// Optional: value-investing research on single stocks (`scry value`).
    #[serde(default)]
    pub value: Option<ValueConfig>,
    /// Optional: mirror strategies into a broker account.
    #[serde(default)]
    pub live: Option<LiveConfig>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueConfig {
    /// Text file of tickers, one per line (`#` comments allowed).
    pub universe_file: PathBuf,
    /// Price history start for the stock universe (momentum needs a year before eval_start).
    pub data_start: NaiveDate,
    /// Re-download a company's SEC filings when the cached copy is older than this.
    #[serde(default = "default_refresh")]
    pub refresh_days: i64,
}

fn default_refresh() -> i64 {
    7
}

impl ValueConfig {
    pub fn symbols(&self) -> Result<Vec<String>> {
        let text = std::fs::read_to_string(&self.universe_file)
            .with_context(|| format!("reading {}", self.universe_file.display()))?;
        Ok(text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.to_uppercase())
            .collect())
    }
}
