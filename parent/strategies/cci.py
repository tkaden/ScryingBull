import backtrader as bt

from parent.data_gathering import data_processing


class CCI(bt.Strategy):
    params = (
        ('upper', 70),
        ('lower', -70),
        ('oneplot', True)
    )

    def __init__(self):
        '''
        Create an dictionary of indicators so that we can dynamically add the
        indicators to the strategy using a loop. This mean the strategy will
        work with any numner of data feeds.
        '''
        self.inds = dict()
        for i, d in enumerate(self.datas):
            self.inds[d] = dict()
            self.inds[d]['cci'] = bt.indicators.CommodityChannelIndex(upperband=self.p.upper, lowerband=self.p.lower)

            # Config plot
            self.inds[d]['cci'].plotinfo.plotname = 'CCI'
            self.inds[d]['cci'].plotinfo.plotlinelabels = True

            if i > 0:  # Check we are not on the first loop of data feed:
                if self.p.oneplot == True:
                    d.plotinfo.plotmaster = self.datas[0]

    def next(self):

        for i, d in enumerate(self.datas):
            dt, dn = self.datetime.date(), d._name
            pos = self.getposition(d).size
            if pos == 0:
                self.buy(data=d, size=(10000 / d.tick_last))
            elif self.inds[d]['cci'] < self.p.lower:
                self.close(data=d)
            # this_data.append([dt, dn, pos, self.inds[d]['cci'], self.p.upper, self.p.lower])
            # gen_data.append(this_data)

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
    cerebro.addstrategy(CCI)

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

    data_processing.export_data('cci')

    # Finally plot the end results
    cerebro.plot(style='line')
