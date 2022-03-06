import backtrader as bt

from src.data_gathering import data_processing


class BOLLStrat(bt.Strategy):
    '''
    Mean reversion bollinger band strategy.

    Entry Critria:
        - Long:
            - Price closes below the lower band
            - Stop Order entry when price crosses back above the lower band
        - Short:
            - Price closes above the upper band
            - Stop order entry when price crosses back below the upper band
    Exit Critria
        - Long/Short: Price touching the median line
    '''

    params = (
        ("period", 10),
        ("devfactor", 2),
        ("size", 10),
        ("debug", False)
    )

    def __init__(self):

        self.boll = bt.indicators.BollingerBands(period=self.p.period, devfactor=self.p.devfactor)
        # self.sx = bt.indicators.CrossDown(self.data.close, self.boll.lines.top)
        # self.lx = bt.indicators.CrossUp(self.data.close, self.boll.lines.bot)

    def next(self):

        orders = self.broker.get_orders_open()

        # Cancel open orders so we can track the median line
        if orders:
            for order in orders:
                self.broker.cancel(order)

        if not self.position:
            if self.data.close > self.boll.lines.top:
                self.sell(exectype=bt.Order.Stop, price=self.boll.lines.top[0], size=self.p.size)
            if self.data.close < self.boll.lines.bot:
                self.buy(exectype=bt.Order.Stop, price=self.boll.lines.bot[0], size=self.p.size)


        else:

            if self.position.size > 0:
                self.sell(exectype=bt.Order.Limit, price=self.boll.lines.mid[0], size=self.p.size)
            else:
                self.buy(exectype=bt.Order.Limit, price=self.boll.lines.mid[0], size=self.p.size)

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
    cerebro.addstrategy(BOLLStrat)

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

    data_processing.export_data('meanDev')

    # Finally plot the end results
    cerebro.plot(style='line')
