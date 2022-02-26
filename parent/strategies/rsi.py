import backtrader as bt

from parent.data_gathering import data_processing


class rsi(bt.Strategy):

    def __init__(self):
        self.inds = dict()
        for i, d in enumerate(self.datas):
            self.inds[d] = dict()
            self.inds[d]['rsi'] = bt.indicators.RSI(self.data.close)

        if i > 0:  # Check we are not on the first loop of data feed:
            if self.p.oneplot == True:
                d.plotinfo.plotmaster = self.datas[0]

    def next(self):
        for i, d in enumerate(self.datas):
            pos = self.getposition(d).size
            if pos == 0:
                if self.inds[d]['rsi'] < 30:
                    self.buy(data=d, size=(10000 / d.tick_last))
            elif self.inds[d]['rsi'] > 70:
                self.close(data=d)

    def notify_trade(self, trade):
        dt = self.data.datetime.date()
        if trade.isclosed:
            data_processing.trade_is_closed(dt, trade)
        else:
            data_processing.trade_is_open(dt, trade)


def run(startcash, watchlist, api, to_date, from_date):
    # Create an instance of cerebro
    cerebro = bt.Cerebro()

    # Add our strategy
    cerebro.addstrategy(rsi)

    stock_list = data_processing.stock_data(watchlist, to_date, from_date)

    # Add the data to Cerebro
    for i in stock_list:
        cerebro.adddata(i)

    # Set our desired cash start
    cerebro.broker.setcash(startcash)
    cerebro.run()

    # Get final portfolio Value
    portvalue = cerebro.broker.getvalue()
    pnl = portvalue - startcash

    # Print out the final result
    print('Final Portfolio Value: ${}'.format(portvalue))
    print('P/L: ${}'.format(pnl))

    data_processing.export_data('rsi')

    # Finally plot the end results
    cerebro.plot(style='line')
