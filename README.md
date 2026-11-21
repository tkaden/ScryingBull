# Scrying Bull

*Scrying bowl*: a fortune-teller's tool for seeing the future. This one tries
hundreds of ways of doing that with markets, and keeps score honestly.

Scrying Bull runs **350+ trading strategies from 11 very different method
families** side by side on the same universe:

1. It **backtests** every strategy on 20 years of real daily prices, split into a
   *train* period (2010–2020) and a held-out *validation* period (2021–today).
2. It **paper trades** every strategy forward, one market day at a time, in an
   append-only ledger.
3. It **publishes** a static leaderboard site that compares backtest results with paper results.

Written in Rust. A full backtest of every strategy over ~20 years × 43 symbols takes about 6 seconds.

| Family | Examples |
|---|---|
| `baseline` | buy & hold, and 20 **random** strategies that form a null distribution |
| `trend` | MA crosses, Donchian/Turtle, MACD, Supertrend, Ichimoku, Hull, KST, Aroon, regression t-stat |
| `momentum` | time-series momentum (raw, vol-scaled, 12-1, multi-horizon), 52-week-high proximity |
| `mean_reversion` | RSI(2) Connors, Bollinger fade, IBS, down-streaks, stochastic, Williams %R, CCI, return z-scores |
| `volatility` | volatility targeting, vol-regime filter, Keltner, chandelier stops, NR4/NR7 |
| `volume` | Klinger, OBV, Chaikin money flow, MFI, volume spikes |
| `statistical` | Kalman-filter trend, Hurst exponent and variance-ratio regime switching, autocorrelation |
| `learning` | online logistic regression, averaged perceptron, k-NN pattern analogues, Markov chains |
| `calendar` | turn-of-month, Halloween, day-of-week |
| `cross_sectional` | relative momentum, dual momentum, long/short momentum, short-term reversal, residual stat-arb, low-vol, risk parity |
| `ensemble` | voting committees, follow-the-leader across families |

## Honesty guarantees

The point is to find out what *actually* works, so the design makes fooling yourself hard:

- **No lookahead, by construction.** Strategies are incremental. They get one bar
  at a time after the close and can't index into the future. Orders fill at the
  **next day's open** with slippage and commission. A test runs every strategy on
  a pure random walk and fails if any of them "finds" an edge.
- **A null distribution.** Twenty random strategies show how good pure luck looks.
  Every result is reported as the share of random strategies it beats. With 350
  strategies, several will look brilliant by chance.
- **Paper trading is append-only.** Each run fills yesterday's orders, marks to
  market and records today's orders. Strategies are replayed to rebuild their
  state, but past paper days are never recomputed. Changing code or data can't
  rewrite the record. A test checks that day-by-day paper trading reproduces a
  backtest exactly.
- **Corporate actions.** Prices are split- and dividend-adjusted. When the vendor
  re-adjusts history, paper positions are rescaled to match, so dividends are
  credited and splits don't look like crashes.
- **New strategies start paper trading the day they're added.** No backfilled "paper" history.

## Quick start

```bash
cargo build --release
./target/release/scry fetch                  # download daily history from Yahoo Finance
./target/release/scry backtest --top 25      # train/validation leaderboard + validation report
./target/release/scry backtest --filter mean_reversion
./target/release/scry paper                  # advance the paper ledger to the latest close
./target/release/scry site                   # build ./site from results + ledger
python3 -m http.server -d site               # view it at http://localhost:8000

./target/release/scry daily                  # fetch + backtest + paper + site in one go
./target/release/scry list                   # every strategy id and description
./target/release/scry analyze volatility.vol_target.20.0.15   # deep-dive tables for one strategy
```

Configuration lives in `scry.toml`: universe, dates, starting cash and trading costs.
All backtests use real market data. A synthetic generator exists only as a unit-test fixture.

## Train / validation

- **Train (2010–2020):** pick strategies and parameters here.
- **Validation (2021–today):** judge those picks here. Never pick on it.

The leaderboard is ranked by train Sharpe, with validation shown beside it. That is
exactly the "what would I have picked, and how did it do?" view. The site plots
train vs validation for every strategy and reports the rank correlation.

What the real data says (as of Nov 2026, 43-symbol universe, 6 bps costs):

- **Rankings persist.** The train→validation Sharpe rank correlation is 0.87. That
  isn't just market exposure: exposure alone correlates 0.07, and among long-only
  strategies the correlation is still 0.71.
- **Very little beats buy & hold.** Only 11 of 336 strategies beat buy & hold's Sharpe
  in training. Only 7 beat it in both periods: cross-sectional momentum and
  volatility targeting. The top 10 by train Sharpe averaged 0.81 in validation,
  below buy & hold's 0.87.
- **Survivorship bias is real here.** On an ETF-only universe (no hindsight stock
  picks), cross-sectional momentum *loses* to buy & hold in training (0.61 vs
  0.79). Its edge on the default universe comes largely from holding stocks known
  in 2026 to have been winners.
- **Volatility targeting is the robust result.** It beats buy & hold's Sharpe on
  both universes and in both periods, with about half the drawdown, but at a lower
  return (Sharpe 1.13 vs 0.87 in validation, CAGR 9% vs 20%).
- **Some families did badly.** Online learners, calendar effects and the
  statistical regime models have median validation Sharpes near or below zero.

Paper trading is the final, fully out-of-sample test. The site's third chart
compares validation results with forward paper results as the ledger grows.

## Deployment (GitHub Pages)

`.github/workflows/daily.yml` runs after every US market close. It fetches prices,
runs the backtests, advances the paper ledger and publishes the site to GitHub
Pages. The ledger is committed to a separate **`ledger` branch**, so the paper
record has an auditable git history and `main` stays clean.

One-time setup: **Settings → Pages → Source: GitHub Actions**. You can also run the
workflow by hand from the Actions tab (`workflow_dispatch`).

## Trading with real money (Alpaca)

`scry live` mirrors one or more strategies into an [Alpaca](https://alpaca.markets)
brokerage account. The broker account copies what those strategies' **paper
accounts** will hold after the next open, scaled to your budget. Orders go out
after the close as market/day orders and fill at the next open, the same timing
as the backtests. Alpaca is commission-free and supports fractional shares, so a
budget of a few hundred dollars works.

**Start on Alpaca's paper environment, then switch one line to go live.**

1. Create an Alpaca account and generate API keys. Paper and live accounts have separate keys.
2. Configure the `[live]` section in `scry.toml`:
   ```toml
   [live]
   mode = "paper"                         # then "live"
   strategies = ["ensemble.family_vote"]  # ids to mirror; weights are averaged
   budget = 500.0
   halt_below = 400.0                     # kill switch
   ```
3. Choose strategies from their *paper* record, not their backtest:
   `scry candidates --min-days 60` ranks them against the random baselines.
4. Try it:
   ```bash
   export ALPACA_KEY_ID=... ALPACA_SECRET_KEY=...
   scry live                    # dry run: prints target weights and the orders it would send
   scry live --execute          # sends orders (paper account)
   scry live --execute --real-money   # required once mode = "live"
   ```
5. To automate it, add the keys as repo **secrets** `ALPACA_KEY_ID` / `ALPACA_SECRET_KEY`.
   Then set the repo **variable** `LIVE_EXECUTE=true`. For real money, also set
   `LIVE_REAL_MONEY=true`. Without these variables the daily job only logs a dry run.

Guardrails:

- **Budget-isolated.** The bot keeps its own cash ledger: the budget, minus its
  buys, plus its sells, reconciled from actual fills. It never spends other cash in
  the account, and never uses margin. Raising `budget` adds the difference.
- **Kill switch.** If the bot's value (its positions plus its cash) falls below
  `halt_below`, it liquidates and stops. It stays stopped until you clear `halted`
  in `live_state.json`.
- **Long-only.** Short targets are dropped.
- **Narrow scope.** It only touches symbols in the universe and only cancels its own
  orders (client ids start with `scry-`). Even so, use an account where you don't
  hand-trade the same tickers.
- **Refuses stale signals.** It won't trade if the paper ledger is more than 5 days old.
- **Audited.** Every executed run is appended to `live_log.jsonl` on the `ledger` branch.

Small accounts are subject to US pattern-day-trader rules. This bot trades at most
once a day per symbol, but fast-flipping strategies can still rack up day trades if
an order fills intraday.

## Adding a strategy

Single-asset methods implement `SymbolStrategy` and return the exposure you want, from −1 to 1:

```rust
struct MyIdea { sma: Sma }
impl SymbolStrategy for MyIdea {
    fn next(&mut self, _date: NaiveDate, bar: &Bar) -> f64 {
        match self.sma.update(bar.close) {
            Some(m) if bar.close > m => 1.0,
            _ => 0.0,
        }
    }
}
```

Whole-universe methods implement `Strategy` and write target weights. They write
`NaN` for "no change", so periodic rebalancers don't trade drift.

Register parameter variants in the family's `specs()` in `src/strategies/<family>.rs`.
**Ids are permanent**, because the paper ledger is keyed on them. A new variant gets a new id.

## Layout

```
src/
  data/          bars, CSV cache, aligned panel; Yahoo + synthetic sources
  indicators.rs  streaming SMA/EMA/RSI/ATR/windows/regression
  strategy.rs    Strategy / SymbolStrategy traits, Spec registry entries
  strategies/    the strategy zoo, one file per family
  engine.rs      fills, costs, sticky targets, gross-exposure cap
  backtest.rs    parallel backtests + null-distribution scoring
  paper.rs       append-only paper ledger
  metrics.rs     CAGR, Sharpe, Sortino, drawdown, yearly returns
  site.rs        static site generator (assets in web/)
  analyze.rs     per-strategy deep dive: curves, drawdowns, parameter grid, cost and universe stress tests
  broker.rs      Alpaca REST client behind a `Broker` trait
  live.rs        mirror paper accounts into a broker account, with guardrails
```

## Limitations and next steps

- Daily bars only, and data comes from Yahoo's unofficial chart API. A paid feed
  (Polygon, Tiingo, Alpaca) would be more reliable.
- Shorts have no borrow fees, and holidays are approximated in the calendar strategies.
- The default universe has survivorship bias (see Train / validation above).
  Point-in-time index membership would fix it. Meanwhile, an ETF-only universe is
  the more honest test.
- Ideas for more families: pairs/cointegration, regime HMMs, options-implied
  signals, macro/rates signals, sentiment/news, genetic programming over the indicator library.
- Only Alpaca is supported. Interactive Brokers or Tradier could implement the same `Broker` trait.
- Dividends paid into the broker account aren't credited to the bot's cash ledger,
  so its reported value slightly understates returns.

Not investment advice.
