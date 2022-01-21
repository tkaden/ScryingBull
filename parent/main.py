from parent.strategies import cci, rsi, meanDev, ema, klinger
from datetime import datetime
from dateutil.relativedelta import relativedelta
from parent.resources import core_constants
from parent.data_gathering import data_processing
import threading

TESTING = False

THREADS = 8
if TESTING:
    THREADS = 1

PROFITS = []
NET = 0

def chunk_list(threads, stock_list):
    chunk_size = int(len(stock_list)/threads)
    return [stock_list[i:i + chunk_size] for i in range(0, len(stock_list), chunk_size)]


def run_thread(stocks, startcash, to_date, from_date):
    for stock in stocks:
        ema.run(stock.strip(), startcash, to_date, from_date)
        for profit in data_processing.profits_list:
            PROFITS.append(profit)
    print(round(sum(PROFITS), 2))

def main():
    # startcash = core_constants.MAX_CASH
    startcash = 100000
    today = datetime.today() + relativedelta(days=1)
    first = datetime.today() - relativedelta(years=2)
    to_date = datetime(today.year, today.month, today.day)
    from_date = datetime(first.year, first.month, first.day)
    stocks = data_processing.pull_watchlist()
    chunks = chunk_list(THREADS, stocks)

    for chunk in chunks:
        threading.Thread(target=run_thread, args=(chunk, startcash, to_date, from_date)).start()


if __name__ == '__main__':
    main()