# === Self-referential structure tests ===
# Cycle detection allows these to work without hitting depth limits

# Self-referential list repr shows ellipsis for cycle
x = []
x.append(x)
assert repr(x) == '[[...]]', 'self-referential list repr shows ellipsis'
assert x == x, 'self-referential list equals itself (identity)'

# Nested self-reference also works
y = []
z = [y]
y.append(z)
assert repr(y) == '[[[...]]]', 'nested self-ref shows ellipsis at cycle point'
assert y == y, 'nested self-ref equals itself'

# Self-referential dict
d = {}
d['self'] = d
assert 'self' in repr(d), 'self-referential dict has self key in repr'
assert d == d, 'self-referential dict equals itself'

# Self-referential tuple via list
a = []
b = []
a.append(b)
b.append(a)
assert repr(a) == '[[[...]]]', 'mutually referential lists show ellipsis'
