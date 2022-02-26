import csv

import backtrader as bt
from parent.data_gathering import data_processing
from parent.resources import core_constants
from parent import main
import matplotlib
import matplotlib.pyplot as plt
matplotlib.use('Agg')

class EmaCross(bt.Strategy):
    # list of parameters which are configurable for the strategy
    params = dict(
        base=1,
        fast=5,  # period for the fast moving average
        mid=20,   # period for the slow moving averag1
        slow=50
    )

    def __init__(self):
            for i, d in enumerate(self.datas):
                self.ema1 = bt.ind.EMA(d.close, period=self.params.base)  # fast moving average
                self.ema2 = bt.ind.EMA(d.close, period=self.params.fast)  # slow moving average
                self.ema3 = bt.ind.EMA(d.close, period=self.params.mid)  # fast moving average
                self.ema4 = bt.ind.EMA(d.close, period=self.params.slow)  # slow moving average
                self.signal = bt.indicators.CrossOver(self. ema3, self.ema4)
                self.momentum = bt.indicators.CrossOver(self.ema1, self.ema2)


    def next(self):
        for i, d in enumerate(self.datas):
            dn = d._name
            dt = self.data.datetime.date()
            if self.getposition(d).size == 0:  # not in the market
                if self.signal > 0:
                    self.buy(d)
            elif self.signal < 0:
                    self.close(d)

            if self.momentum > 0:
                data_processing.gaining_momentum(dt, dn, d.close)
            elif self.momentum < 0:
                data_processing.losing_momentum(dt, dn, d.close)

    def notify_trade(self, trade):
        dt = self.data.datetime.date()
        if trade.isclosed:
            data_processing.trade_is_closed(dt, trade)
        else:
            data_processing.trade_is_open(dt, trade)


def run(stock, startcash, to_date, from_date):

        cerebro = bt.Cerebro()
        cerebro.addstrategy(EmaCross)
        stock_data = data_processing.stock_data(stock, to_date, from_date)
        cerebro.adddata(stock_data)
        cerebro.broker.setcash(startcash)
        cerebro.run()

        # Get final portfolio Value
        portvalue = cerebro.broker.getvalue()
        cash = cerebro.broker.getcash()
        pnl = portvalue - startcash
        percent_gain = 0
        if cash != startcash:
            percent_gain = pnl/(startcash-cash)
        csv.writer(core_constants.pnl_file_write).writerow([stock, round(pnl, 2), round(percent_gain*100, 2)])

        cerebro.plot(style='line')