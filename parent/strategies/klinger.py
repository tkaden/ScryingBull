import backtrader as bt

from parent.data_gathering import data_processing


class KlingerOsc(bt.Indicator):
    '''
    - KVO and Signal Crossovers: Since we have two lines, an obvious option would be to go long when the KVO crossed above the signal line and go short when the KVO line crosses under the signal line.
    - Zero Crossovers: When the KVO or Signal lines (you pick) crosses over the zero line.
    - Divergence: When price is making a new high or low but the KVO line is not making a new high/low.
    - Measure the strength of the movement, aka the “volume force” as confirmation to determine if a price movement has conviction.
    '''

    lines = ('sig', 'kvo')

    params = (('kvoFast', 34), ('kvoSlow', 55), ('sigPeriod', 13))

    def __init__(self):
        self.plotinfo.plotyhlines = [0]
        self.addminperiod(55)

        self.data.hlc3 = (self.data.high + self.data.low + self.data.close) / 3
        # This works - Note indexing should be () rather than []
        # https://www.backtrader.com/docu/concepts.html#lines-delayed-indexing
        self.data.sv = bt.If((self.data.hlc3(0) - self.data.hlc3(-1)) / self.data.hlc3(-1) >= 0, self.data.volume,
                             -self.data.volume)
        self.lines.kvo = bt.indicators.EMA(self.data.sv, period=self.p.kvoFast) - bt.indicators.EMA(self.data.sv,
                                                                                                    period=self.p.kvoSlow)
        self.lines.sig = bt.indicators.EMA(self.lines.kvo, period=self.p.sigPeriod)

        self.signal = bt.indicators.CrossOver(self.lines.kvo, self.lines.sig)


class Klinger(bt.Strategy):

    def __init__(self):
        self.KOsc = KlingerOsc(self.data)

    # def next(self):
    #     if self.getposition == 0:
    #         if self.KOsc.lines.kvo > self.KOsc.lines.sig:
    #             self.buy(size=(10))
    #     else:
    #         if self.KOsc.lines.kvo < self.KOsc.lines.sig:
    #             self.close()

    # if(self.signal == 1):
    #     self.buy(size=100)
    # if(self.signal == -1):
    #     self.close(size=100)


def run(startcash, watchlist, api, to_date, from_date):
    # Create an instance of cerebro
    cerebro = bt.Cerebro()

    # Add our strategy
    cerebro.addstrategy(Klinger)

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

    data_processing.export_data('klinger')

    # Finally plot the end results
    cerebro.plot(style='line')
