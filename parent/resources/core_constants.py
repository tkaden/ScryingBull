# Watchlist File for stocks used in starts
WATCHLIST = open("../parent/resources/watchlist.txt", "r")

WATCHLIST_TEST = open("../parent/resources/watchlist_test.txt", "r")

# Use max cash when wanting to generate signals only
MAX_CASH = float("inf")

BUY = "Signal Triggered: Buy"
SELL = "Signal Triggered: Sell"
RISING = "Signal Triggered: Gaining Momentum"
FALLING = "Signal Triggered: Losing Momentum"

buy_file = open("../parent/data_gathering/signal_data/buy.csv", "+w")
sell_file = open("../parent/data_gathering/signal_data/sell.csv", "+w")
rising_file = open("../parent/data_gathering/signal_data/rising.csv", "+w")
falling_file = open("../parent/data_gathering/signal_data/falling.csv", "+w")

pnl_file_write = open("../parent/data_gathering/signal_data/pnl.csv", "a")
pnl_file_read = open("../parent/data_gathering/signal_data/pnl.csv", "r")
