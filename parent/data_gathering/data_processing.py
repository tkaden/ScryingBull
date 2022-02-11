import pandas as pd
import numpy as np
import yfinance as yf
import backtrader as bt
from parent.resources import core_constants
from datetime import datetime
from pandas.tseries.offsets import BDay
from parent import main
from parent.resources import config
import csv

gen_data = []
signals = []
profits_list = []

def export_data(strat):
    profits = 0

    if len(gen_data) > 0:
        stock_name = gen_data[0][1]
        for trade in gen_data:
            profits = profits + trade[2]
            trade.append(round(profits, 2))

        profits_list.append(gen_data[len(gen_data)-1][2])

        backtrader_df = pd.DataFrame(gen_data)
        backtrader_df.columns = ["Date", "Stock", "NetTrade", "NetProfit"]
        backtrader_df.to_csv("../parent/data_gathering/backtest_data/" + str(strat) + "_" + stock_name + ".csv")
        del backtrader_df

        write_signals(signals)
        gen_data.clear(), signals.clear()

    else:
        print("No Data")

def trade_is_closed(dt, trade):
    data = gen_data
    print('{} {} Closed: PnL Gross {}, Net {}'.format(
        dt,
        trade.data._name,
        round(trade.pnl, 2),
        round(trade.pnlcomm, 2)))
    val = round(trade.pnl, 2)
    data.append([format(dt), trade.data._name, val])
    signals.append([format(dt), trade.data._name, val, core_constants.SELL])
    return data

def trade_is_open(dt, trade):
    print('{} {} Open: Port value {}'.format(
        dt,
        trade.data._name,
        round(trade.value, 2)))
    val = round(trade.pnl, 2)
    signals.append([format(dt), trade.data._name, val, core_constants.BUY])

def losing_momentum(dt, name, open):
    signals.append([format(dt), name, open, core_constants.FALLING])

def gaining_momentum(dt, name, open):
    signals.append([format(dt), name, open, core_constants.RISING])

def stock_data(stock, to_date, from_date):
    return bt.feeds.Quandl(
        dataname=stock,
        fromdate=from_date,
        todate=to_date,
        buffered=True,
        apikey=config.quandlapi
    )

def filter_signals(signal_list):
    filtered = []
    if is_weekday():
        buis_day = str(datetime.today()).split()[0]
    else:
        buis_day = str(datetime.today() - BDay(1)).split()[0]
    for i in range(len(signal_list)):
        if signal_list[i][0] == buis_day:
            filtered.append(signal_list[i])
    return filtered

def write_signals(signals):
    filtered_signals = filter_signals(signals)
    if len(filtered_signals) > 0:

        for signal in filtered_signals:
            if signal[3] == core_constants.BUY:
                csv.writer(core_constants.buy_file).writerow(signal)
            elif signal[3] == core_constants.SELL:
                csv.writer(core_constants.sell_file).writerow(signal)
            elif signal[3] == core_constants.RISING:
                csv.writer(core_constants.rising_file).writerow(signal)
            elif signal[3] == core_constants.FALLING:
                csv.writer(core_constants.falling_file).writerow(signal)
            else:
                print("ERROR: Failure to writing signals")

        filtered_signals.clear()

def pull_watchlist():
    if main.TESTING:
        return core_constants.WATCHLIST_TEST.readlines()
    else:
        return core_constants.WATCHLIST.readlines()

def is_weekday():
    week_num = datetime.today().weekday()
    if week_num < 5:
        return True
    else:
        return False
