use anyhow::{bail, Result};
use chrono::{NaiveDate, Utc};
use clap::{Parser, Subcommand};
use scrying_bull::backtest::{self, BacktestResult};
use scrying_bull::config::Config;
use scrying_bull::data::{yahoo, Panel};
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
    /// Mirror the configured strategies into an Alpaca account. Dry run unless --execute.
    Live {
        #[arg(long, default_value = "ledger/ledger.json")]
        ledger: PathBuf,
        #[arg(long, default_value = "ledger/live_state.json")]
        state: PathBuf,
        /// Actually send orders (otherwise only print the plan).
        #[arg(long)]
        execute: bool,
        /// Required in addition to --execute when `live.mode = "live"`.
        #[arg(long)]
        real_money: bool,
    },
    /// Deep-dive analytics for one strategy (JSON tables for dashboards).
    Analyze {
        /// Strategy id, e.g. volatility.vol_target.20.0.15
        id: String,
        #[arg(long, default_value = "results/analysis")]
        out: PathBuf,
    },
    /// Rank strategies by their *paper* record, to choose what to trade live.
    Candidates {
        #[arg(long, default_value = "ledger/ledger.json")]
        ledger: PathBuf,
        /// Minimum paper trading days.
        #[arg(long, default_value_t = 60)]
        min_days: usize,
        #[arg(long, default_value_t = 20)]
        top: usize,
    },
    /// fetch + backtest + paper + site: what the daily job runs.
    Daily {
        #[arg(long, default_value = "ledger/ledger.json")]
        ledger: PathBuf,
        #[arg(long, default_value = "site")]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load(&cli.config)?;
    match cli.cmd {
        Cmd::Fetch => fetch(&cfg),
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
        Cmd::Live {
            ledger,
            state,
            execute,
            real_money,
        } => run_live(&cfg, &ledger, &state, execute, real_money),
        Cmd::Analyze { id, out } => {
            let panel = load_panel(&cfg)?;
            let alt = if cfg.robustness_universe.is_empty() {
                None
            } else {
                Some(Panel::load(
                    &cfg.data_dir,
                    &cfg.robustness_universe,
                    cfg.data_start,
                )?)
            };
            scrying_bull::analyze::analyze(
                &cfg,
                &panel,
                alt.as_ref(),
                &registry(),
                &id,
                &out.join(&id),
            )
        }
        Cmd::Candidates {
            ledger,
            min_days,
            top,
        } => candidates(&Ledger::load(&ledger)?, min_days, top),
        Cmd::Daily { ledger, out } => {
            fetch(&cfg)?;
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
    let r = backtest::run_all(
        specs,
        &panel,
        cfg.eval_start,
        cfg.validation_start,
        cfg.cash,
        &cfg.costs,
    );
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
    println!("\nRanked by TRAIN Sharpe (what you'd pick); VALIDATION is the held-out check.");
    println!(
        "{:<46} {:>7} | {:>7} {:>7} {:>7} {:>6}",
        "strategy", "train S", "val S", "val CAGR", "val DD", ">rand"
    );
    for r in results.iter().take(top) {
        let v = &r.validation;
        println!(
            "{:<46} {:>7.2} | {:>7.2} {:>7.1}% {:>6.1}% {:>5.0}%",
            r.id,
            r.train.sharpe,
            v.sharpe,
            v.cagr * 100.0,
            v.max_drawdown * 100.0,
            r.beats_random_validation * 100.0
        );
    }
    let rep = backtest::validation_report(results);
    println!(
        "\nvalidation report over {} strategies:\n  train->validation Sharpe rank correlation: {:.2}\n  top 10 by train: mean validation Sharpe {:.2}\n  all strategies: median validation Sharpe {:.2}",
        rep.strategies, rep.rank_corr, rep.top10_val_sharpe, rep.median_val_sharpe
    );
    if let Some(b) = rep.buy_hold_val_sharpe {
        println!("  buy & hold: validation Sharpe {b:.2}");
    }
    println!("  beat 95% of random in BOTH periods: {}", rep.robust.len());
    for id in rep.robust.iter().take(15) {
        println!("    {id}");
    }
}

fn write_json<T: serde::Serialize>(path: &Path, v: &T) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, serde_json::to_string(v)?)?;
    Ok(())
}

fn run_live(
    cfg: &Config,
    ledger_path: &Path,
    state_path: &Path,
    execute: bool,
    real_money: bool,
) -> Result<()> {
    use scrying_bull::broker::{Alpaca, Broker};
    use scrying_bull::live::{self, LiveState, Mode};

    let Some(lc) = &cfg.live else {
        bail!("no [live] section in {}", "scry.toml")
    };
    if execute && lc.mode == Mode::Live && !real_money {
        bail!("live.mode = \"live\" trades REAL MONEY; pass --real-money as well as --execute to confirm");
    }
    let ledger = Ledger::load(ledger_path)?;
    let Some(last) = ledger.last_date else {
        bail!("paper ledger is empty; run `scry paper` first")
    };
    let age = (Utc::now().date_naive() - last).num_days();
    if age > 5 {
        bail!(
            "paper ledger is {age} days old (last date {last}); refusing to trade on stale signals"
        );
    }
    let weights = live::blend(&ledger, &lc.strategies)?;
    println!(
        "mode: {:?} | mirroring {} | budget ${:.2}",
        lc.mode,
        lc.strategies.join(", "),
        lc.budget
    );
    println!("target weights after next open:");
    for (s, w) in weights.iter().filter(|(_, w)| **w > 0.0) {
        println!("  {s:<6} {:>6.2}%", w * 100.0);
    }

    let broker = match Alpaca::from_env(lc.mode == Mode::Live) {
        Ok(b) => b,
        Err(e) if !execute => {
            println!("\n({e:#}; showing targets only)");
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let mut state = LiveState::load(state_path)?;
    if let Some(why) = &state.halted {
        println!(
            "\nHALTED: {why}\nNo orders sent. Delete `halted` in {} to resume.",
            state_path.display()
        );
        return Ok(());
    }
    let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    for line in state.reconcile(lc, &broker, &now)? {
        println!("{line}");
    }
    let acct = broker.account()?;
    if acct.blocked {
        bail!("Alpaca reports the account is blocked from trading");
    }
    if broker.market_open()? {
        println!(
            "note: market is open now, so orders fill immediately rather than at the next open"
        );
    }
    let positions = broker.positions()?;
    let prices = ledger
        .ref_close
        .iter()
        .map(|(s, (_, c))| (s.clone(), *c))
        .collect();
    let p = live::plan(
        lc,
        &weights,
        &cfg.universe,
        &positions,
        &prices,
        state.bot_cash,
        acct.cash,
    );
    println!(
        "\naccount equity ${:.2} | bot cash ${:.2} | managed value ${:.2}",
        acct.equity, state.bot_cash, p.managed_value
    );
    for n in &p.notes {
        println!("{n}");
    }
    if p.orders.is_empty() {
        println!("no orders needed");
    }
    for o in &p.orders {
        println!("  {o:?}");
    }
    if !execute {
        println!("\ndry run: pass --execute to send these orders");
        return Ok(());
    }
    let tag = Utc::now().format("%Y%m%d%H%M%S").to_string();
    let fills = live::execute(&broker, &p, &tag)?;
    if p.halted {
        state.halted = Some(format!(
            "{now}: managed value ${:.2} fell below ${:.2}",
            p.managed_value, lc.halt_below
        ));
    }
    let today = Utc::now().date_naive();
    state.history.retain(|(d, _)| *d != today);
    state
        .history
        .push((today, (p.managed_value * 100.0).round() / 100.0));
    state.save(state_path)?;
    let log = serde_json::json!({ "time": now, "mode": lc.mode, "strategies": lc.strategies, "plan": p, "results": fills });
    let log_path = state_path.with_file_name("live_log.jsonl");
    use std::io::Write;
    writeln!(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?,
        "{log}"
    )?;
    Ok(())
}

fn candidates(ledger: &Ledger, min_days: usize, top: usize) -> Result<()> {
    use scrying_bull::metrics::compute;
    use scrying_bull::strategies::baseline::RANDOM_PREFIX;
    let mut rows: Vec<(String, scrying_bull::metrics::Metrics)> = ledger
        .accounts
        .iter()
        .filter(|(_, a)| a.active && a.equity.len() > min_days)
        .map(|(id, a)| (id.clone(), compute(&a.equity)))
        .collect();
    if rows.is_empty() {
        let max = ledger
            .accounts
            .values()
            .map(|a| a.equity.len().saturating_sub(1))
            .max()
            .unwrap_or(0);
        println!("no strategy has {min_days}+ paper days yet (longest: {max}). Use --min-days to lower the bar, but short records are mostly noise.");
        return Ok(());
    }
    let mut null: Vec<f64> = rows
        .iter()
        .filter(|(id, _)| id.starts_with(RANDOM_PREFIX))
        .map(|r| r.1.sharpe)
        .collect();
    null.sort_by(f64::total_cmp);
    rows.retain(|(id, _)| !id.starts_with(RANDOM_PREFIX));
    rows.sort_by(|a, b| b.1.sharpe.total_cmp(&a.1.sharpe));
    println!(
        "{:<46} {:>5} {:>8} {:>7} {:>7} {:>7}",
        "strategy (paper record)", "days", "return", "Sharpe", "MaxDD", ">rand"
    );
    for (id, m) in rows.iter().take(top) {
        let beats =
            null.iter().filter(|s| **s < m.sharpe).count() as f64 / null.len().max(1) as f64;
        println!(
            "{id:<46} {:>5} {:>7.2}% {:>7.2} {:>6.1}% {:>6.0}%",
            m.days - 1,
            m.total_return * 100.0,
            m.sharpe,
            m.max_drawdown * 100.0,
            beats * 100.0
        );
    }
    println!(
        "\nA strategy that doesn't beat ~95% of the random baselines has not shown an edge yet."
    );
    Ok(())
}
