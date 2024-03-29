import threading
from datetime import datetime

from dateutil.relativedelta import relativedelta

from src.data_gathering import data_processing
from src.strategies import ema

TESTING = True

THREADS = 8
if TESTING:
    THREADS = 1

PROFITS = []
NET = 0


def chunk_list(threads, stock_list):
    chunk_size = int(len(stock_list) / threads)
    return [stock_list[i:i + chunk_size] for i in range(0, len(stock_list), chunk_size)]


def run_thread(stocks, startcash, to_date, from_date):
    for stock in stocks:
        ema.run(stock.strip(), startcash, to_date, from_date)
        for profit in data_processing.profits_list:
            PROFITS.append(profit)
    print(round(sum(PROFITS), 2))


def main():
    startcash = 100000
    today = datetime.today() - relativedelta(days=1)
    first = datetime.today() - relativedelta(years=8)
    to_date = datetime(today.year, today.month, today.day)
    from_date = datetime(first.year, first.month, first.day)
    stocks = data_processing.pull_watchlist()
    chunks = chunk_list(THREADS, stocks)

    if TESTING == False:
        for chunk in chunks:
            threading.Thread(target=run_thread, args=(chunk, startcash, to_date, from_date)).start()
    else:
        ema.run(stocks[0].strip(), startcash, to_date, from_date)
        for profit in data_processing.profits_list:
            PROFITS.append(profit)
        print(round(sum(PROFITS), 2))


def lambda_func(event, context):
    main()


if __name__ == '__main__':
    main()
