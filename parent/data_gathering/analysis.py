from parent.resources import core_constants
import csv
import pandas as pd

wins = []
losses = []
w_perc = 0
l_perc = 0
w_net = 0
l_net = 0
df = pd.read_csv(core_constants.pnl_file_read)
df.columns = ['Stock', 'Net', 'Percent Gain']

for i in df['Percent Gain']:
    if i > 0:
        wins.append(i)
        w_perc += i
    else:
        losses.append(i)
        l_perc += i

for j in df['Net']:
    if j > 0:
        w_net += j
    else:
        l_net += j


print('Number of Wins: ' + str(len(wins)) + ' (' + str(round((len(wins)/(len(wins)+len(losses)))*100, 2)) + '%)')
print('Number of Losses: ' + str(len(losses)) + ' (' + str(round((len(losses)/(len(wins)+len(losses)))*100, 2)) + '%)')
print('Avg Win %: ' + str(round(w_perc/len(wins), 2)))
print('Avg Loss %: ' + str(round(l_perc/len(losses), 2)))
print('Total Net Win: ' + str(round(w_net, 2)))
print('Total Net Loss: ' + str(round(l_net, 2)))
print('Avg Net Win: ' + str(round(w_net/len(wins), 2)))
print('Avg Net Loss: ' + str(round(l_net/len(losses), 2)))


