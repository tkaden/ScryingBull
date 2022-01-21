import csv

import backtrader as bt
from datetime import datetime
import matplotlib

from dateutil.relativedelta import relativedelta

from parent.resources import core_constants


class EmaCross(bt.Strategy):
    # list of parameters which are configurable for the strategy
    params = dict(
        base=1,
        fast=5,  # period for the fast moving average
        mid=21,   # period for the slow moving average
        slow=200
    )

    def __init__(self):
            for i, d in enumerate(self.datas):
                self.ema1 = bt.ind.EMA(d.close, period=self.params.base)  # fast moving average
                self.ema2 = bt.ind.EMA(d.close, period=self.params.fast)  # slow moving average
                self.ema3 = bt.ind.EMA(d.close, period=self.params.mid)  # fast moving average
                self.ema4 = bt.ind.EMA(d.close, period=self.params.slow)  # slow moving average
                self.signal = bt.indicators.CrossOver(self.ema4, self.ema3)
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


#Variable for our starting cash
startcash = 10000

#Create an instance of cerebro
cerebro = bt.Cerebro()

#Add our strategy
cerebro.addstrategy(EmaCross)


today = datetime.today() + relativedelta(days=1)
first = datetime.today() - relativedelta(years=2)
to_date = datetime(today.year, today.month, today.day)
from_date = datetime(first.year, first.month, first.day)
#Get Apple data from Yahoo Finance.
data = bt.feeds.YahooFinanceData(
    dataname='AAPL',
    fromdate=from_date,
    todate=to_date,
    buffered=True
    )

#Add the data to Cerebro
cerebro.adddata(data)

# Set our desired cash start
cerebro.broker.setcash(startcash)

# Run over everything
cerebro.run()

#Get final portfolio Value
portvalue = cerebro.broker.getvalue()
pnl = portvalue - startcash
pnl_list = []
pnl_list.append(round(pnl, 2))
csv.writer(core_constants.pnl_file_write).writerow(pnl_list)

#Print out the final result
print('Final Portfolio Value: ${}'.format(portvalue))
print('P/L: ${}'.format(round(pnl, 2)))

#Finally plot the end results
cerebro.plot(style='line')