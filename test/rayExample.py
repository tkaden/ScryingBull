from datetime import datetime

import backtrader as bt


class EmaCross(bt.Strategy):
    # list of parameters which are configurable for the strategy
    params = dict(
        pfaaast=1,
        pfast=5,  # period for the fast moving average
        pslow=13,  # period for the slow moving average
        pslooow=20
    )

    def __init__(self):
        for i, d in enumerate(self.datas):
            dn = d._name
            if dn == 'AAPL':
                self.ema1 = bt.ind.EMA(d.close, period=self.p.pfast)  # fast moving average
                self.ema2 = bt.ind.EMA(d.close, period=self.p.pslow)  # slow moving average
                self.ema3 = bt.ind.EMA(d.close, period=self.p.pfaaast)  # fast moving average
                self.ema4 = bt.ind.EMA(d.close, period=self.p.pslooow)  # slow moving average
                self.signal = bt.indicators.CrossOver(self.ema3, self.ema4)
                # Lagging trend

    def next(self):
        for i, d in enumerate(self.datas):
            dn = d._name
            if dn == 'AAPL':
                if self.getposition(d).size == 0:  # not in the market
                    #                    print(d.datetime.date(0))
                    #                    print(self.getposition(d).size)
                    if self.ema3 < (1.9 * self.ema4) and self.ema1.lines.ema[0] >= self.ema2.lines.ema[
                        0]:  # if fast crosses slow to the upside
                        self.buy(d)  # enter long

                    elif self.ema3.lines.ema[0] <= (1.9 * self.ema4.lines.ema[0]) and self.ema1.lines.ema[0] >= \
                            self.ema2.lines.ema[0]:  # if VIX fast crosses slow to the downside
                        self.buy(d)

                else:
                    if self.ema3 > (1.9 * self.ema4) and (d.close - d.open) < 0:
                        self.close(d)
                    if self.ema1.lines.ema[0] < self.ema2.lines.ema[0]:  # in the market & cross to the downside
                        self.close(d)  # close long position


# Variable for our starting cash
startcash = 10000

# Create an instance of cerebro
cerebro = bt.Cerebro()

# Add our strategy
cerebro.addstrategy(EmaCross)

# Get Apple data from Yahoo Finance.
data = bt.feeds.Quandl(
    dataname='AAPL',
    fromdate=datetime(2016, 1, 1),
    todate=datetime(2017, 1, 1),
    buffered=True,
    apikey="a15HDnYHzQj_1GJ-WZrS"
)

# Add the data to Cerebro
cerebro.adddata(data)

# Set our desired cash start
cerebro.broker.setcash(startcash)

# Run over everything
cerebro.run()

# Get final portfolio Value
portvalue = cerebro.broker.getvalue()
pnl = portvalue - startcash

# Print out the final result
print('Final Portfolio Value: ${}'.format(portvalue))
print('P/L: ${}'.format(pnl))

# Finally plot the end results
cerebro.plot(style='line')
