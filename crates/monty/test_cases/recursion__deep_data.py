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

# === Deep nesting tests (within limits) ===
# These test that moderate nesting works correctly without hitting limits

# Moderately nested list (depth 20 - well under any limit)
nested_list = []
for _ in range(20):
    nested_list = [nested_list]
r = repr(nested_list)
# 20 wrappings around [] = 21 opening brackets
assert r.count('[') == 21, 'nested list has 21 opening brackets'
assert r.count(']') == 21, 'nested list has 21 closing brackets'

# Moderately nested tuple
nested_tuple: tuple = ()  # type: ignore
for _ in range(20):
    nested_tuple = (nested_tuple,)
r = repr(nested_tuple)
# 20 wrappings around () = 21 opening parens
assert r.count('(') == 21, 'nested tuple has 21 opening parens'

# Moderately nested dict
nested_dict: dict = {}  # type: ignore
for _ in range(20):
    nested_dict = {'a': nested_dict}
r = repr(nested_dict)
# 20 wrappings = 20 'a' keys
assert r.count("'a'") == 20, 'nested dict has 20 keys'

# Moderately nested set (frozenset for hashability)
nested_set: frozenset = frozenset()  # type: ignore
for _ in range(10):
    nested_set = frozenset([nested_set])
r = repr(nested_set)
assert 'frozenset' in r, 'nested frozenset repr contains frozenset'

# Deep equality comparison works
list1 = []
list2 = []
for _ in range(20):
    list1 = [list1]
    list2 = [list2]
assert list1 == list2, 'deeply nested equal lists compare equal'

# Deep inequality comparison works
list3 = []
list4 = []
for _ in range(19):
    list3 = [list3]
    list4 = [list4]
list3 = [list3]
list4 = [list4, 1]  # Different length at deepest level
assert list3 != list4, 'deeply nested unequal lists compare unequal'
