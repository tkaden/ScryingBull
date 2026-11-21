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
