"""rpds-py, the rpds crate's persistent data structures: HashTrieMap (insert, remove and discard,
update, convert, fromkeys, get, views and their set operations, equality and hashing, keys whose
hashes collide, keys whose __eq__ raises), HashTrieSet (the set operations, comparisons), List,
Queue and Stack (each operation, and what it raises when empty), pickling (and that Queue and
Stack refuse it), structural sharing (the old versions unchanged), large ones, and the structures
shared between threads. What a hash trie iterates in depends on the hashes, which are the
platform's (32-bit in the guest): the maps and sets are printed sorted, their reprs only when they
hold one item. tests/reference.py runs it with rpds-py's own wheel.
"""
import pickle
import threading

import rpds
from rpds import HashTrieMap, HashTrieSet, List, Queue, Stack


def s(x):
    return sorted(x, key=repr)


print("all", rpds.__all__)

# ── HashTrieMap ─────────────────────────────────────────────────────────────────────────────────
m = HashTrieMap({"a": 1, "b": 2}, c=3)
m2 = m.insert("d", 4).insert("a", 10)
print("map", s(m.items()), s(m2.items()), len(m), len(m2), "a" in m2, "z" in m2)
print("map get", m2["a"], m2.get("z"), m2.get("z", 0), m.get("a"))
print("map remove", s(m2.remove("a").items()), s(m2.discard("z").items()), m2.discard("z") == m2)
try:
    m.remove("z")
except KeyError as e:
    print("map remove missing", repr(e))
try:
    m["z"]
except KeyError as e:
    print("map getitem missing", repr(e))
print("map update", s(m.update({"e": 5}, [("f", 6)], HashTrieMap(g=7)).items()))
print("map convert", HashTrieMap.convert(m) is m, s(HashTrieMap.convert({"x": 1}).items()))
print("map fromkeys", s(HashTrieMap.fromkeys("abc").items()), s(HashTrieMap.fromkeys([1, 2], 0).items()))
print("map repr", repr(HashTrieMap()), repr(HashTrieMap({"k": [1, 2]})), repr(HashTrieMap({1: HashTrieMap()})))
print("map views", s(m.keys()), s(m.values()), s(m.items()), len(m.keys()), "a" in m.keys(), ("a", 1) in m.items())
print("map keys ops", s(m.keys() & {"a", "z"}), s(m.keys() | {"z"}), s(m.keys() & HashTrieMap(a=0, y=0).keys()))
print("map items ops", s(m.items() & {("a", 1), ("b", 9)}), s(m.items() | {("z", 0)}))
print("map eq", m == {"a": 1, "b": 2, "c": 3}, m == HashTrieMap(c=3, b=2, a=1), m != m2, m == 1)
print("map hash", hash(m) == hash(HashTrieMap(c=3, b=2, a=1)), hash(HashTrieMap()) == hash(HashTrieMap()))
try:
    hash(HashTrieMap(a=[1]))
except TypeError as e:
    print("map hash unhashable", type(e).__name__)
print("map dict", dict(m) == {"a": 1, "b": 2, "c": 3}, s(iter(m)))


class Collide:
    """Every instance the same hash: the trie keeps them in one bucket."""

    def __init__(self, n):
        self.n = n

    def __hash__(self):
        return 42

    def __eq__(self, other):
        return isinstance(other, Collide) and self.n == other.n

    def __repr__(self):
        return f"Collide({self.n})"


cm = HashTrieMap((Collide(i), i) for i in range(20))
print("collide", len(cm), [cm[Collide(i)] for i in range(0, 20, 5)], Collide(20) in cm, len(cm.remove(Collide(3))))


class BadEq:
    def __hash__(self):
        return 7

    def __eq__(self, other):
        raise RuntimeError("no comparing")


print("bad eq alone", len(HashTrieMap({BadEq(): 1})))

# ── HashTrieSet ─────────────────────────────────────────────────────────────────────────────────
a, b = HashTrieSet([1, 2, 3, 4]), HashTrieSet([3, 4, 5])
print("set", s(a), len(a), 3 in a, 9 in a, s(a.insert(9)), s(a.discard(1)), s(a.discard(42)))
try:
    a.remove(42)
except KeyError as e:
    print("set remove missing", repr(e))
print("set ops", s(a | b), s(a & b), s(a - b), s(a ^ b))
print("set methods", s(a.union(b)), s(a.intersection(b)), s(a.difference(b)), s(a.symmetric_difference(b)),
      s(a.update([8], {9})))
print("set compare", HashTrieSet([3, 4]) <= a, HashTrieSet([3, 4]) < a, a >= a, a > a, a == {1, 2, 3, 4},
      a.isdisjoint(HashTrieSet([7])) if hasattr(a, "isdisjoint") else "no isdisjoint")
print("set repr", repr(HashTrieSet()), repr(HashTrieSet(["one"])))
print("set hash", hash(HashTrieSet([1, 2])) == hash(HashTrieSet([2, 1])))

# ── List, Queue, Stack ──────────────────────────────────────────────────────────────────────────
lst = List([1, 2, 3])
print("list", list(lst), repr(lst), len(lst), lst.first, list(lst.rest), list(lst.push_front(0)), list(lst.drop_first()))
print("list more", list(List(1, 2, 3)), list(reversed(lst)), lst == List([1, 2, 3]), lst != List([3]),
      hash(lst) == hash(List([1, 2, 3])), repr(List()))
for name, f in (("first", lambda: List().first), ("drop_first", lambda: List().drop_first())):
    try:
        f()
    except IndexError as e:
        print("list empty", name, repr(e))
q = Queue([1, 2, 3])
print("queue", list(q), repr(q), len(q), q.peek(), list(q.enqueue(4)), list(q.dequeue()), q.is_empty(), Queue().is_empty())
print("queue more", list(Queue(1, 2)), q == Queue([1, 2, 3]), q != Queue(), repr(Queue()))
for name, f in (("peek", lambda: Queue().peek()), ("dequeue", lambda: Queue().dequeue())):
    try:
        f()
    except (IndexError, TypeError) as e:
        print("queue empty", name, repr(e))
st = Stack([1, 2, 3])
print("stack", list(st), repr(st), len(st), st.peek(), list(st.push(0)), list(st.pop()), st == Stack([1, 2, 3]))
for name, f in (("peek", lambda: Stack().peek()), ("pop", lambda: Stack().pop())):
    try:
        f()
    except (IndexError, TypeError) as e:
        print("stack empty", name, repr(e))

for x in (q, st):
    try:
        pickle.dumps(x)
    except TypeError as e:
        print("pickle", type(x).__name__, e)

# ── pickling, sharing, size ─────────────────────────────────────────────────────────────────────
for x in (m, a, lst, HashTrieMap({"nested": HashTrieSet([List([1])])})):
    y = pickle.loads(pickle.dumps(x))
    print("pickle", type(y).__name__, y == x)
base = HashTrieMap((i, i * i) for i in range(1000))
versions = [base]
for i in range(5):
    versions.append(versions[-1].insert(f"k{i}", i).remove(i))
print("versions", [len(v) for v in versions], base[3], 3 in versions[-1], versions[-1]["k4"])
big = HashTrieMap()
for i in range(100000):
    big = big.insert(i, str(i))
print("big", len(big), big[99999], sum(big.keys()), len(HashTrieSet(range(100000)) - HashTrieSet(range(0, 100000, 2))))
big_list = List(range(50000))
print("big list", len(big_list), big_list.first, sum(big_list))

# ── threads: one map read and built on in several at once ───────────────────────────────────────
shared = HashTrieMap((i, i) for i in range(5000))
results = {}


def work(t):
    local = shared
    for i in range(2000):
        local = local.insert((t, i), i).discard(i)
    results[t] = (len(local), sum(v for k, v in local.items() if isinstance(k, tuple)), len(shared))


threads = [threading.Thread(target=work, args=(t,)) for t in range(4)]
for t in threads:
    t.start()
for t in threads:
    t.join()
print("threads", sorted(results.items()))
print("done")
