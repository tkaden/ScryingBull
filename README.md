# Scrying Bull

*Scrying bowl*: a fortune-teller's tool for seeing the future. This one tries
hundreds of ways of doing that with markets, and keeps score honestly.

Scrying Bull runs **350+ trading strategies from 11 very different method
families** side by side on the same universe:

1. It **backtests** every strategy.
2. It **paper trades** every strategy forward, one market day at a time, in an
   append-only ledger.
3. It **publishes** a static leaderboard site that compares backtest results with paper results.

Written in Rust. A full backtest of every strategy over ~20 years × 43 symbols takes a few seconds.

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

# Offline: synthetic data (for development only; says nothing about real markets)
./target/release/scry synth
./target/release/scry backtest --top 20

# Real data from Yahoo Finance
./target/release/scry fetch
./target/release/scry backtest --filter mean_reversion
./target/release/scry paper          # advance the paper ledger to the latest close
./target/release/scry site           # build ./site from results + ledger
python3 -m http.server -d site       # view it at http://localhost:8000

./target/release/scry daily          # fetch + backtest + paper + site in one go
./target/release/scry list           # every strategy id and description
```

Configuration lives in `scry.toml`: universe, dates, starting cash and trading costs.

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
  broker.rs      Alpaca REST client behind a `Broker` trait
  live.rs        mirror paper accounts into a broker account, with guardrails
```

## Limitations and next steps

- Daily bars only, and data comes from Yahoo's unofficial chart API. A paid feed
  (Polygon, Tiingo, Alpaca) would be more reliable.
- Shorts have no borrow fees, and holidays are approximated in the calendar strategies.
- The universe is fixed and has survivorship bias: today's large caps looked
  great in hindsight. Point-in-time index membership would fix this.
- Ideas for more families: pairs/cointegration, regime HMMs, options-implied
  signals, macro/rates signals, sentiment/news, genetic programming over the indicator library.
- Only Alpaca is supported. Interactive Brokers or Tradier could implement the same `Broker` trait.
- Dividends paid into the broker account aren't credited to the bot's cash ledger,
  so its reported value slightly understates returns.

Not investment advice.
