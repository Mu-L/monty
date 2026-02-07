# Plan: Guard Against Deep Nesting Overflow in Container Operations

## Problem Summary

`deep.py` creates 10,000 nested lists (each list containing the previous one):
```python
x = []
for _ in range(10000):
    x = [x]
x  # Triggers repr on return
```

This causes a **Rust stack overflow** when `repr()` is called because:
1. Each list is a distinct object (different HeapId), so cycle detection doesn't trigger
2. The recursive `py_repr_fmt` calls exhaust the Rust stack (~10,000 call frames)

### Affected Operations

All recursive container traversal operations are vulnerable:

| Operation | Current Signature | Issue |
|-----------|------------------|-------|
| `py_repr_fmt` | `(..., heap_ids: &mut AHashSet<HeapId>)` | Has cycle detection, no depth limit |
| `py_str` | `(&Heap, &Interns) -> Cow<str>` | Calls py_repr, same issue |
| `py_repr` | `(&Heap, &Interns) -> Cow<str>` | Wrapper around py_repr_fmt, needs Result |
| `py_eq` | `(&mut Heap, &Interns) -> bool` | No depth limit |
| `py_cmp` | `(&mut Heap, &Interns) -> Option<Ordering>` | No depth limit |
| `compute_hash_if_immutable` | `(&mut Heap, &Interns) -> Option<u64>` | Recurses for tuples |
| **`dec_ref`** | `(&mut self, id: HeapId)` | **CRITICAL: Recursive deallocation overflows on drop** |

### Critical: `dec_ref` Recursion

Even if `repr(deep_obj)` raises `RecursionError` and the script continues, when `deep_obj` goes out of scope, `dec_ref` will recursively free all nested objects, causing a **crash** (stack overflow). This must be fixed with an iterative algorithm.

## Solution Design

### Approach: Add Recursion Depth Tracking to Heap

Store a recursion counter in `Heap` using `Cell<u16>` to allow incrementing with immutable borrows (needed for `py_repr_fmt`). Use RAII guard pattern for automatic cleanup.

**Why this approach:**
- Minimal API changes - Heap is already passed to all affected operations
- `Cell` allows mutation through `&Heap` (needed for `py_repr_fmt` which takes `&Heap`)
- RAII guard ensures counter is decremented even on early returns/errors
- Matches CPython's approach (global recursion counter per thread state)
- Single counter shared across all operations catches pathological cases (repr calling eq calling repr)

### Key Design Decisions

1. **Counter type**: `Cell<u16>` - allows mutation through shared ref, u16 matches parser limit type
2. **Default limit**: 200 (matching `MAX_NESTING_DEPTH` for consistency with parser)
3. **Debug limit**: 35 (matching debug parser limit due to larger stack frames)
4. **Error type**: Return `RecursionError` via existing error mechanisms
5. **Check location**: At container entry points before recursing into children

## Implementation Plan

### Step 0: Convert `dec_ref` to Iterative (CRITICAL)

**File: `crates/monty/src/heap.rs`**

**Problem:** Current `dec_ref` recursively calls itself for each child:
```rust
for child_id in child_ids {
    self.dec_ref(child_id);  // RECURSIVE - will overflow!
}
```

**Solution:** Convert to iterative using a work list (similar to `collect_garbage`):
```rust
pub fn dec_ref(&mut self, id: HeapId) {
    let mut work_list = vec![id];

    while let Some(current_id) = work_list.pop() {
        let slot = self.entries.get_mut(current_id.index()).expect("Heap::dec_ref: slot missing");
        let entry = slot.as_mut().expect("Heap::dec_ref: object already freed");

        if entry.refcount > 1 {
            entry.refcount -= 1;
            continue;
        }

        // refcount == 1, free the value
        if let Some(value) = slot.take() {
            self.free_list.push(current_id);

            if let Some(ref data) = value.data {
                self.tracker.on_free(|| data.py_estimate_size());
            }

            if let Some(mut data) = value.data {
                // Push children to work_list instead of recursing
                data.py_dec_ref_ids(&mut work_list);
            }
        }
    }
}
```

**Note:** This reuses the existing `py_dec_ref_ids` method which pushes HeapIds to a Vec. The change is purely in how we process them (iteratively vs recursively).

### Step 1: Add Recursion Tracking to Heap

**File: `crates/monty/src/heap.rs`**

Add field to Heap struct (after `allocations_since_gc`):
```rust
/// Tracks recursion depth for container operations (repr, eq, cmp, hash).
/// Uses Cell to allow incrementing through &Heap for py_repr_fmt.
/// Prevents stack overflow from deeply nested (but not cyclic) structures.
data_recursion_depth: Cell<u16>,
```

Add constant at module level (near top of file):
```rust
/// Maximum recursion depth for data structure operations (repr, eq, cmp, hash).
/// Matches `MAX_NESTING_DEPTH` from parser for consistency.
#[cfg(not(debug_assertions))]
pub const MAX_DATA_RECURSION_DEPTH: u16 = 200;
#[cfg(debug_assertions)]
pub const MAX_DATA_RECURSION_DEPTH: u16 = 35;
```

Add RAII guard struct (before Heap impl block):
```rust
/// RAII guard that decrements data recursion depth on drop.
///
/// Created by `Heap::enter_data_recursion()` when entering recursive
/// container operations. Automatically decrements depth when dropped,
/// ensuring correct cleanup even on early returns or errors.
pub struct DataRecursionGuard<'a> {
    depth: &'a Cell<u16>,
}

impl Drop for DataRecursionGuard<'_> {
    fn drop(&mut self) {
        // Saturating sub in case of underflow (shouldn't happen with correct usage)
        self.depth.set(self.depth.get().saturating_sub(1));
    }
}
```

Add methods to Heap impl block:
```rust
/// Enters a recursive data structure operation (repr, eq, hash, etc).
///
/// Returns a guard that decrements the depth on drop. Returns
/// `Err(ResourceError::Recursion)` if depth limit exceeded.
///
/// # Example
/// ```ignore
/// let _guard = heap.enter_data_recursion()?;
/// // recursive operation here
/// // guard drops automatically, decrementing depth
/// ```
pub fn enter_data_recursion(&self) -> Result<DataRecursionGuard<'_>, ResourceError> {
    let current = self.data_recursion_depth.get();
    if current >= MAX_DATA_RECURSION_DEPTH {
        return Err(ResourceError::Recursion {
            limit: MAX_DATA_RECURSION_DEPTH as usize,
            depth: current as usize,
        });
    }
    self.data_recursion_depth.set(current + 1);
    Ok(DataRecursionGuard { depth: &self.data_recursion_depth })
}
```

Update Heap::new() and serialization to initialize/handle the new field.

### Step 2: Update py_repr_fmt

**Files: `crates/monty/src/value.rs`, `crates/monty/src/types/*.rs`**

**Challenge:** `py_repr_fmt` returns `std::fmt::Result` which can't carry `RecursionError`.

**Solution:** Change return type to a custom error enum that can carry both:

In `crates/monty/src/types/py_trait.rs`, add:
```rust
/// Error type for repr operations that can fail due to recursion or formatting.
#[derive(Debug)]
pub enum ReprError {
    Fmt(std::fmt::Error),
    Recursion(ResourceError),
}

impl From<std::fmt::Error> for ReprError {
    fn from(e: std::fmt::Error) -> Self { Self::Fmt(e) }
}
impl From<ResourceError> for ReprError {
    fn from(e: ResourceError) -> Self { Self::Recursion(e) }
}
```

Update `py_repr_fmt` signature:
```rust
fn py_repr_fmt(
    &self,
    f: &mut impl Write,
    heap: &Heap<impl ResourceTracker>,
    heap_ids: &mut AHashSet<HeapId>,
    interns: &Interns,
) -> Result<(), ReprError>;  // Changed from std::fmt::Result
```

In `Value::py_repr_fmt` for the `Ref` case:
```rust
Self::Ref(id) => {
    if heap_ids.contains(id) {
        // Existing cycle detection...
        Ok(())
    } else {
        let _guard = heap.enter_data_recursion()?;  // Returns ReprError::Recursion on overflow
        heap_ids.insert(*id);
        let result = heap.get(*id).py_repr_fmt(f, heap, heap_ids, interns);
        heap_ids.remove(id);
        result
    }
}
```

**Update `py_repr` and `py_str` signatures** (major change):

Current signatures return `Cow<'static, str>` which cannot propagate errors:
```rust
fn py_repr(&self, heap: &Heap<...>, interns: &Interns) -> Cow<'static, str>;
fn py_str(&self, heap: &Heap<...>, interns: &Interns) -> Cow<'static, str>;
```

Change to return `Result` to propagate `RecursionError`:
```rust
fn py_repr(&self, heap: &Heap<...>, interns: &Interns) -> Result<Cow<'static, str>, ResourceError>;
fn py_str(&self, heap: &Heap<...>, interns: &Interns) -> Result<Cow<'static, str>, ResourceError>;
```

**Scope of changes:** This affects all call sites of `repr()` and `str()`:
- Exception formatting
- Print statements
- String formatting (f-strings, % formatting)
- Logging/debugging
- Error messages

All these call sites must be updated to handle the `Result`.

### Step 3: Update py_eq and py_cmp

**Files: `crates/monty/src/types/py_trait.rs`, `crates/monty/src/value.rs`, `crates/monty/src/types/list.rs`, etc.**

Change trait signature in `PyTrait`:
```rust
fn py_eq(&self, other: &Self, heap: &mut Heap<impl ResourceTracker>, interns: &Interns)
    -> Result<bool, ResourceError>;  // Was: bool

fn py_cmp(&self, other: &Self, heap: &mut Heap<impl ResourceTracker>, interns: &Interns)
    -> Result<Option<Ordering>, ResourceError>;  // Was: Option<Ordering>
```

In container implementations (list, tuple, dict, set, dataclass, namedtuple):
```rust
fn py_eq(&self, other: &Self, heap: &mut Heap<impl ResourceTracker>, interns: &Interns)
    -> Result<bool, ResourceError>
{
    if self.items.len() != other.items.len() {
        return Ok(false);
    }
    let _guard = heap.enter_data_recursion()?;
    for (i1, i2) in self.items.iter().zip(&other.items) {
        if !i1.py_eq(i2, heap, interns)? {
            return Ok(false);
        }
    }
    Ok(true)
}
```

For leaf types (Str, Bytes, Range, Slice, Path):
```rust
fn py_eq(&self, other: &Self, _heap: &mut Heap<impl ResourceTracker>, _interns: &Interns)
    -> Result<bool, ResourceError>
{
    Ok(self.inner == other.inner)  // Wrap existing logic in Ok()
}
```

Update all call sites in the VM and elsewhere to handle the Result.

### Step 4: Update compute_hash_if_immutable

**File: `crates/monty/src/heap.rs`**

Change to return `Result<Option<u64>, ResourceError>` or check depth before recursive hash calls.

### Step 5: Add Tests

**File: `crates/monty/test_cases/recursion__deep_data.py`**

Test that deeply nested structures raise RecursionError:
```python
x = []
for _ in range(1000):
    x = [x]
repr(x)
"""
TRACEBACK:
Traceback (most recent call last):
  File "recursion__deep_data.py", line 4, in <module>
    repr(x)
    ~~~~~~~
RecursionError: maximum recursion depth exceeded
"""
```

## Files to Modify

1. `crates/monty/src/heap.rs` - **Convert `dec_ref` to iterative**, add recursion tracking field, guard, and methods
2. `crates/monty/src/value.rs` - Update py_repr, py_repr_fmt, py_str, py_eq, py_cmp implementations
3. `crates/monty/src/types/py_trait.rs` - Update trait signatures (py_repr, py_str, py_eq, py_cmp, py_repr_fmt)
4. `crates/monty/src/types/list.rs` - Update py_eq, py_repr_fmt
5. `crates/monty/src/types/tuple.rs` - Update py_eq, py_repr_fmt
6. `crates/monty/src/types/dict.rs` - Update py_eq, py_repr_fmt
7. `crates/monty/src/types/set.rs` - Update py_eq for set/frozenset
8. `crates/monty/src/types/dataclass.rs` - Update py_eq
9. `crates/monty/src/types/namedtuple.rs` - Update py_eq
10. `crates/monty/src/bytecode/vm/*.rs` - Update call sites for py_repr, py_str, py_eq that now return Result
11. `crates/monty/src/builtins/*.rs` - Update repr(), str() builtins and error formatting
12. `crates/monty/test_cases/` - Add test for deep nesting RecursionError

## Design Notes

**Thread Safety:** The use of `Cell<u16>` for recursion depth implies `Heap` is single-threaded. This is correct - Monty VMs are not shared across threads. If threaded parallelism is ever added, this would need to change to `AtomicU16`.

**Future Configuration:** The hardcoded limits (200/35) could eventually be exposed via a `sys.setrecursionlimit` equivalent, but this is out of scope for this fix.

## Verification

1. Run `make test-ref-count-panic` to ensure all existing tests pass
2. Run deep.py and verify it raises RecursionError instead of crashing
3. **Test that deep structures don't crash on drop** - create deep structure, let it go out of scope
4. Test self-referential structures still work (cycle detection preserved)
5. Test normal nested structures within limits work correctly
6. Run `make format-rs && make lint-rs` to ensure code quality

## Alternative Considered: Iterative Algorithms

Could convert recursive operations to use explicit stacks (heap-allocated). This avoids Rust stack overflow entirely but:
- Much more complex implementation
- Harder to read/maintain
- Would need to rewrite multiple operations

The depth-limit approach is simpler and matches how CPython handles this.
