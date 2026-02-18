# === reverse=True ===
assert sorted([3, 1, 2], reverse=True) == [3, 2, 1], 'reverse int list'
assert sorted([1, 2, 3], reverse=False) == [1, 2, 3], 'reverse=False is default'
assert sorted([], reverse=True) == [], 'reverse empty'
assert sorted('cab', reverse=True) == ['c', 'b', 'a'], 'reverse string iterable'

# === key function (lambda) ===
assert sorted([3, 1, 2], key=lambda x: -x) == [3, 2, 1], 'key negate'
assert sorted(['banana', 'apple', 'cherry'], key=lambda s: len(s)) == ['apple', 'banana', 'cherry'], 'key len'
assert sorted([(1, 'b'), (2, 'a'), (1, 'a')], key=lambda t: t[1]) == [(2, 'a'), (1, 'a'), (1, 'b')], 'key element'

# === key function (builtin) ===
assert sorted([3, -1, 2], key=abs) == [-1, 2, 3], 'key abs'

# === key + reverse ===
assert sorted([3, 1, 2], key=lambda x: -x, reverse=True) == [1, 2, 3], 'key + reverse'

# === key=None (default) ===
assert sorted([3, 1, 2], key=None) == [1, 2, 3], 'key None default'
assert sorted([3, 1, 2], key=None, reverse=True) == [3, 2, 1], 'key None + reverse'
