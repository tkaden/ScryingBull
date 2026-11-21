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
    /// Backtests trade and are scored from this date.
    pub eval_start: NaiveDate,
    /// Starting capital for every backtest and paper account.
    pub cash: f64,
    pub costs: Costs,
    pub universe: Vec<String>,
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
