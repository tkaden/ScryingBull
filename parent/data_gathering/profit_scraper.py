from parent.resources import core_constants

list_profits = []
for line in core_constants.pnl_file_read.readlines():
    if line.strip() == '':
        continue
    item = line.replace('\n', '')
    list_profits.append(float(item))
print(list_profits)
print(round(sum(list_profits), 2))
