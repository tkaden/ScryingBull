use anyhow::{bail, Result};
use chrono::{Duration, NaiveDate, Utc};
use clap::{Parser, Subcommand};
use scrying_bull::backtest::{self, BacktestResult};
use scrying_bull::config::Config;
use scrying_bull::data::{synthetic, yahoo, Panel};
use scrying_bull::paper::{self, Ledger};
use scrying_bull::site;
use scrying_bull::strategies::registry;
use scrying_bull::strategy::Spec;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "scry",
    version,
    about = "Backtest, paper trade and publish hundreds of trading strategies."
)]
struct Cli {
    /// Config file.
    #[arg(long, short, default_value = "scry.toml", global = true)]
    config: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Download daily bars for the universe from Yahoo Finance into the data cache.
    Fetch,
    /// Write synthetic data for the universe (offline development; NOT real markets).
    Synth {
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },
    /// List registered strategies.
    List {
        /// Only ids containing this text.
        #[arg(long)]
        filter: Option<String>,
    },
    /// Backtest strategies and print a leaderboard.
    Backtest {
        #[arg(long)]
        filter: Option<String>,
        #[arg(long, default_value_t = 25)]
        top: usize,
        /// Write full results (with equity curves) as JSON.
        #[arg(long, default_value = "results/backtest.json")]
        out: PathBuf,
    },
    /// Advance the paper-trading ledger to the latest data.
    Paper {
        #[arg(long, default_value = "ledger/ledger.json")]
        ledger: PathBuf,
    },
    /// Build the static results website.
    Site {
        #[arg(long, default_value = "results/backtest.json")]
        backtest: PathBuf,
        #[arg(long, default_value = "ledger/ledger.json")]
        ledger: PathBuf,
        #[arg(long, default_value = "site")]
        out: PathBuf,
    },
    /// fetch + backtest + paper + site: what the daily job runs.
    Daily {
        #[arg(long, default_value = "ledger/ledger.json")]
        ledger: PathBuf,
        #[arg(long, default_value = "site")]
        out: PathBuf,
        /// Use synthetic data instead of fetching (for local dry runs).
        #[arg(long)]
        synthetic: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load(&cli.config)?;
    match cli.cmd {
        Cmd::Fetch => fetch(&cfg),
        Cmd::Synth { seed } => synth(&cfg, seed),
        Cmd::List { filter } => {
            let specs = filtered(filter.as_deref())?;
            for s in &specs {
                println!("{:<52} {:<16} {}", s.id, s.family, s.description);
            }
            eprintln!("{} strategies", specs.len());
            Ok(())
        }
        Cmd::Backtest { filter, top, out } => {
            let results = run_backtest(&cfg, &filtered(filter.as_deref())?)?;
            print_leaderboard(&results, top);
            write_json(&out, &results)
        }
        Cmd::Paper { ledger } => run_paper(&cfg, &ledger).map(|_| ()),
        Cmd::Site {
            backtest,
            ledger,
            out,
        } => {
            let results: Vec<BacktestResult> =
                serde_json::from_str(&std::fs::read_to_string(&backtest)?)?;
            let ledger = Ledger::load(&ledger)?;
            site::build(&out, &cfg, &results, &ledger)?;
            println!("site written to {}", out.display());
            Ok(())
        }
        Cmd::Daily {
            ledger,
            out,
            synthetic,
        } => {
            if synthetic {
                synth(&cfg, 42)?;
            } else {
                fetch(&cfg)?;
            }
            let results = run_backtest(&cfg, &registry())?;
            print_leaderboard(&results, 15);
            write_json(Path::new("results/backtest.json"), &results)?;
            let l = run_paper(&cfg, &ledger)?;
            site::build(&out, &cfg, &results, &l)?;
            println!("site written to {}", out.display());
            Ok(())
        }
    }
}

fn filtered(filter: Option<&str>) -> Result<Vec<Spec>> {
    let specs: Vec<Spec> = registry()
        .into_iter()
        .filter(|s| filter.is_none_or(|f| s.id.contains(f)))
        .collect();
    if specs.is_empty() {
        bail!("no strategies match");
    }
    Ok(specs)
}

fn fetch(cfg: &Config) -> Result<()> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .try_proxy_from_env(true)
        .build();
    let mut failed = Vec::new();
    for sym in &cfg.universe {
        match yahoo::fetch(&agent, sym, cfg.data_start) {
            Ok(s) if !s.bars.is_empty() => {
                println!(
                    "{sym:<6} {} bars, {} .. {}",
                    s.bars.len(),
                    s.bars[0].date,
                    s.bars.last().unwrap().date
                );
                s.save(&cfg.data_dir)?;
            }
            Ok(_) => failed.push(format!("{sym}: empty")),
            Err(e) => failed.push(format!("{e:#}")),
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    for f in &failed {
        eprintln!("warning: {f}");
    }
    std::fs::create_dir_all(&cfg.data_dir)?;
    std::fs::write(cfg.data_dir.join(site::SOURCE_FILE), "yahoo")?;
    // Tolerate a few failures (cached data is kept), but not a total outage.
    if failed.len() * 2 > cfg.universe.len() {
        bail!(
            "{} of {} symbols failed to download",
            failed.len(),
            cfg.universe.len()
        );
    }
    Ok(())
}

fn synth(cfg: &Config, seed: u64) -> Result<()> {
    let end = Utc::now().date_naive() - Duration::days(1);
    for s in synthetic::generate(&cfg.universe, cfg.data_start, end, seed) {
        s.save(&cfg.data_dir)?;
    }
    std::fs::write(cfg.data_dir.join(site::SOURCE_FILE), "synthetic")?;
    println!(
        "wrote synthetic data for {} symbols to {}",
        cfg.universe.len(),
        cfg.data_dir.display()
    );
    Ok(())
}

fn load_panel(cfg: &Config) -> Result<Panel> {
    let p = Panel::load(&cfg.data_dir, &cfg.universe, cfg.data_start)?;
    println!(
        "loaded {} symbols x {} days ({} .. {})",
        p.symbols.len(),
        p.len(),
        p.dates[0],
        p.dates[p.len() - 1]
    );
    Ok(p)
}

fn run_backtest(cfg: &Config, specs: &[Spec]) -> Result<Vec<BacktestResult>> {
    let panel = load_panel(cfg)?;
    let t = Instant::now();
    let r = backtest::run_all(specs, &panel, cfg.eval_start, cfg.cash, &cfg.costs);
    println!(
        "backtested {} strategies in {:.1}s",
        r.len(),
        t.elapsed().as_secs_f64()
    );
    Ok(r)
}

fn run_paper(cfg: &Config, path: &Path) -> Result<Ledger> {
    let panel = load_panel(cfg)?;
    let mut ledger = Ledger::load(path)?;
    if let (Some(last), Some(end)) = (ledger.last_date, panel.dates.last()) {
        if *end < last {
            bail!("data ends {end}, before the ledger's last date {last}; refusing to rewind");
        }
    }
    let rep = paper::advance(&mut ledger, &panel, &registry(), cfg.cash, &cfg.costs);
    for a in &rep.adjustments {
        println!("corporate action: {a}");
    }
    println!(
        "paper: {} new day(s), {} accounts advanced, {} opened; ledger at {}",
        rep.new_days,
        rep.updated,
        rep.opened,
        ledger
            .last_date
            .map_or("-".into(), |d: NaiveDate| d.to_string())
    );
    ledger.save(path)?;
    Ok(ledger)
}

fn print_leaderboard(results: &[BacktestResult], top: usize) {
    println!(
        "\n{:<46} {:>7} {:>7} {:>7} {:>8} {:>6} {:>7}",
        "strategy", "CAGR", "Sharpe", "MaxDD", "1y ret", "trades", ">rand"
    );
    for r in results.iter().take(top) {
        let m = &r.metrics;
        println!(
            "{:<46} {:>6.1}% {:>7.2} {:>6.1}% {:>7.1}% {:>6} {:>6.0}%",
            r.id,
            m.cagr * 100.0,
            m.sharpe,
            m.max_drawdown * 100.0,
            r.last_year.total_return * 100.0,
            r.trades,
            r.beats_random * 100.0
        );
    }
    if let Some(bh) = results.iter().find(|r| r.id == "baseline.buy_hold") {
        let rank = results.iter().position(|r| r.id == bh.id).unwrap() + 1;
        println!(
            "\nbuy & hold: CAGR {:.1}%, Sharpe {:.2} (rank {rank} of {})",
            bh.metrics.cagr * 100.0,
            bh.metrics.sharpe,
            results.len()
        );
    }
}

fn write_json<T: serde::Serialize>(path: &Path, v: &T) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, serde_json::to_string(v)?)?;
    Ok(())
}
