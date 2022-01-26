# Scrying Bull

Scrying Bull is a play on the term 'Scrying Bowl', a tool used by fortune-tellers to gain insights or clairvoyance. 

This application works as a platform to easily develop and back-test trading strategies on a list of stocks. This gives the user insights on potentially hundreds of buy/sell signals in a matter of seconds.

The end goal for this project is to deploy to AWS and run the report daily, generating KPIs in a Tableau dashboard allowing the user to make daily trade decisions based on the performance of their trading strategies.

## Limitations

This application is not meant for day traders, but rather those who make trades on a longer timeline. Ideally, this would be used for making decisions on 1-3 month option contracts.

Right now there is no user interface for the code. In order to run the code, you'll have to run main.py in terminal or in the IDE.

Recently, Yahoo Finance changed their API which broke the data extraction for the stocks. This is first up to be fixed.

## Upcoming Improvements
- Fix data extraction.
- Improve how stocks are fed into the strategies in order to be more dynamic and test on specific sectors, index, individual stocks defined by the user.
- Deploy code into AWS and develop a scheduler to run the code.
- Improve data storage. Potentially using DynamoDB or an RDS instance.
- Output data to a Tableau dashboard running on Tableau Server.
- Continue to develop more trading strategies.

## Installation

Use the package manager [pip](https://pip.pypa.io/en/stable/) to install foobar.

```bash
pip install -r req.txt
```

## Usage
- Adjust the stock tickers you want to run by adding them to 'watchlist.txt'.
- In main.py, you can adjust the strategy used by changing the strategy in run_thread method.
- Run main method