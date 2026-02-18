//! Implementation of the sorted() builtin function.
//!
//! Supports `sorted(iterable, *, key=None, reverse=False)` matching Python's
//! signature. The `key` argument can be any callable, including user-defined
//! functions and lambdas — these are called via `VM::call_sync`.

use std::cmp::Ordering;

use crate::{
    args::{ArgValues, KwargsValues},
    bytecode::VM,
    defer_drop_mut,
    exception_private::{ExcType, RunError, RunResult, SimpleException},
    heap::{Heap, HeapData},
    intern::Interns,
    resource::{DepthGuard, ResourceTracker},
    types::{CallOutcome, List, MontyIter, PyTrait},
    value::Value,
};

/// Implementation of `sorted()` with full VM context for calling key functions.
///
/// Supports the `key` and `reverse` keyword arguments. When a `key` function is
/// provided, each element is passed through it via `VM::call_sync`, which allows
/// user-defined functions (lambdas, closures) to be used as key functions.
pub fn builtin_sorted<T: ResourceTracker>(vm: &mut VM<'_, '_, T>, args: ArgValues) -> RunResult<CallOutcome> {
    let (positional, kwargs) = args.into_parts();
    defer_drop_mut!(positional, vm);

    // Parse key and reverse kwargs
    let (key_arg, reverse_arg) = parse_sorted_kwargs(kwargs, vm.heap, vm.interns)?;

    let positional_len = positional.len();
    if positional_len != 1 {
        if let Some(k) = key_arg {
            k.drop_with_heap(vm.heap);
        }
        return Err(SimpleException::new_msg(
            ExcType::TypeError,
            format!("sorted expected 1 argument, got {positional_len}"),
        )
        .into());
    }

    // Convert reverse to bool (default false)
    let reverse = if let Some(v) = reverse_arg {
        let result = v.py_bool(vm.heap, vm.interns);
        v.drop_with_heap(vm.heap);
        result
    } else {
        false
    };

    // Handle key function: None means no key function
    let key_fn = match key_arg {
        Some(v) if matches!(v, Value::None) => {
            v.drop_with_heap(vm.heap);
            None
        }
        other => other,
    };

    // Collect items from iterable
    let iterable = positional.next().unwrap();
    let iter = MontyIter::new(iterable, vm.heap, vm.interns)?;
    let items = iter.collect(vm.heap, vm.interns)?;

    // Sort and return
    let sorted = sort_items(items, key_fn.as_ref(), reverse, vm)?;
    let heap_id = vm.heap.allocate(HeapData::List(List::new(sorted)))?;

    // Clean up key function
    if let Some(k) = key_fn {
        k.drop_with_heap(vm.heap);
    }

    Ok(CallOutcome::Value(Value::Ref(heap_id)))
}

/// Basic sorted() without VM context (no kwargs support).
///
/// Used via `call_basic` when sorted() is called as a key function from
/// `list.sort()`, where no VM is available.
pub fn builtin_sorted_basic(
    heap: &mut Heap<impl ResourceTracker>,
    args: ArgValues,
    interns: &Interns,
) -> RunResult<Value> {
    let (positional, kwargs) = args.into_parts();
    defer_drop_mut!(positional, heap);

    kwargs.not_supported_yet("sorted", heap)?;

    let positional_len = positional.len();
    if positional_len != 1 {
        return Err(SimpleException::new_msg(
            ExcType::TypeError,
            format!("sorted expected 1 argument, got {positional_len}"),
        )
        .into());
    }

    let iterable = positional.next().unwrap();
    let iter = MontyIter::new(iterable, heap, interns)?;
    let items = iter.collect(heap, interns)?;

    let sorted = sort_items_basic(items, false, heap, interns)?;
    let heap_id = heap.allocate(HeapData::List(List::new(sorted)))?;
    Ok(Value::Ref(heap_id))
}

/// Parses `key` and `reverse` keyword arguments from kwargs.
///
/// Consumes the `KwargsValues`, extracting optional `key` and `reverse` values.
/// Any unexpected keyword argument produces a TypeError.
fn parse_sorted_kwargs(
    kwargs: KwargsValues,
    heap: &mut Heap<impl ResourceTracker>,
    interns: &Interns,
) -> RunResult<(Option<Value>, Option<Value>)> {
    if kwargs.is_empty() {
        return Ok((None, None));
    }

    let mut key_val: Option<Value> = None;
    let mut reverse_val: Option<Value> = None;

    for (k, value) in kwargs {
        let keyword_name = k.as_either_str(&*heap);
        k.drop_with_heap(heap);

        let Some(keyword_name) = keyword_name else {
            value.drop_with_heap(heap);
            // Clean up already-parsed values
            if let Some(v) = key_val {
                v.drop_with_heap(heap);
            }
            if let Some(v) = reverse_val {
                v.drop_with_heap(heap);
            }
            return Err(ExcType::type_error("keywords must be strings"));
        };

        let key_str = keyword_name.as_str(interns);
        if key_str == "key" {
            if let Some(old) = key_val.replace(value) {
                old.drop_with_heap(heap);
            }
        } else if key_str == "reverse" {
            if let Some(old) = reverse_val.replace(value) {
                old.drop_with_heap(heap);
            }
        } else {
            value.drop_with_heap(heap);
            if let Some(v) = key_val {
                v.drop_with_heap(heap);
            }
            if let Some(v) = reverse_val {
                v.drop_with_heap(heap);
            }
            return Err(ExcType::type_error(format!(
                "'{key_str}' is an invalid keyword argument for sorted()"
            )));
        }
    }

    Ok((key_val, reverse_val))
}

/// Sorts items using an optional key function and reverse flag.
///
/// When a key function is provided, calls it on each element via `VM::call_sync`,
/// then uses index-based permutation sorting (same pattern as `do_list_sort` in
/// `list.rs`). This ensures stability and correct reference counting.
fn sort_items<T: ResourceTracker>(
    mut items: Vec<Value>,
    key_fn: Option<&Value>,
    reverse: bool,
    vm: &mut VM<'_, '_, T>,
) -> RunResult<Vec<Value>> {
    // Step 1: Compute key values if key function provided
    let key_values: Option<Vec<Value>> = if let Some(key) = key_fn {
        let mut keys: Vec<Value> = Vec::with_capacity(items.len());
        for item in &items {
            let elem = item.clone_with_heap(vm.heap);
            let key_args = ArgValues::One(elem);
            match vm.call_sync(key, key_args) {
                Ok(key_value) => keys.push(key_value),
                Err(e) => {
                    // Clean up computed keys and items on error
                    for k in keys {
                        k.drop_with_heap(vm.heap);
                    }
                    for item in items {
                        item.drop_with_heap(vm.heap);
                    }
                    return Err(e);
                }
            }
        }
        Some(keys)
    } else {
        None
    };

    // Step 2: Sort indices based on items or key values
    let len = items.len();
    let mut indices: Vec<usize> = (0..len).collect();
    let mut sort_error: Option<RunError> = None;
    let guard = std::cell::RefCell::new(DepthGuard::default());

    if let Some(ref keys) = key_values {
        indices.sort_by(|&a, &b| {
            if sort_error.is_some() {
                return Ordering::Equal;
            }
            if let Err(e) = vm.heap.check_time() {
                sort_error = Some(e.into());
                return Ordering::Equal;
            }
            match keys[a].py_cmp(&keys[b], vm.heap, &mut guard.borrow_mut(), vm.interns) {
                Ok(Some(ord)) => {
                    if reverse {
                        ord.reverse()
                    } else {
                        ord
                    }
                }
                Ok(None) => {
                    sort_error = Some(ExcType::type_error(format!(
                        "'<' not supported between instances of '{}' and '{}'",
                        keys[a].py_type(vm.heap),
                        keys[b].py_type(vm.heap)
                    )));
                    Ordering::Equal
                }
                Err(e) => {
                    sort_error = Some(e.into());
                    Ordering::Equal
                }
            }
        });
    } else {
        indices.sort_by(|&a, &b| {
            if sort_error.is_some() {
                return Ordering::Equal;
            }
            if let Err(e) = vm.heap.check_time() {
                sort_error = Some(e.into());
                return Ordering::Equal;
            }
            match items[a].py_cmp(&items[b], vm.heap, &mut guard.borrow_mut(), vm.interns) {
                Ok(Some(ord)) => {
                    if reverse {
                        ord.reverse()
                    } else {
                        ord
                    }
                }
                Ok(None) => {
                    sort_error = Some(ExcType::type_error(format!(
                        "'<' not supported between instances of '{}' and '{}'",
                        items[a].py_type(vm.heap),
                        items[b].py_type(vm.heap)
                    )));
                    Ordering::Equal
                }
                Err(e) => {
                    sort_error = Some(e.into());
                    Ordering::Equal
                }
            }
        });
    }

    // Clean up key values
    if let Some(keys) = key_values {
        for k in keys {
            k.drop_with_heap(vm.heap);
        }
    }

    // Check for sort error
    if let Some(err) = sort_error {
        for item in items {
            item.drop_with_heap(vm.heap);
        }
        return Err(err);
    }

    // Step 3: Rearrange items in sorted order using index permutation
    let mut sorted_items: Vec<Value> = Vec::with_capacity(len);
    for &i in &indices {
        sorted_items.push(std::mem::replace(&mut items[i], Value::Undefined));
    }

    Ok(sorted_items)
}

/// Sorts items without a key function (basic path without VM).
///
/// Used by `builtin_sorted_basic` for the no-kwargs case.
fn sort_items_basic(
    mut items: Vec<Value>,
    reverse: bool,
    heap: &mut Heap<impl ResourceTracker>,
    interns: &Interns,
) -> RunResult<Vec<Value>> {
    let len = items.len();
    let mut indices: Vec<usize> = (0..len).collect();
    let mut sort_error: Option<RunError> = None;
    let guard = std::cell::RefCell::new(DepthGuard::default());

    indices.sort_by(|&a, &b| {
        if sort_error.is_some() {
            return Ordering::Equal;
        }
        if let Err(e) = heap.check_time() {
            sort_error = Some(e.into());
            return Ordering::Equal;
        }
        match items[a].py_cmp(&items[b], heap, &mut guard.borrow_mut(), interns) {
            Ok(Some(ord)) => {
                if reverse {
                    ord.reverse()
                } else {
                    ord
                }
            }
            Ok(None) => {
                sort_error = Some(ExcType::type_error(format!(
                    "'<' not supported between instances of '{}' and '{}'",
                    items[a].py_type(heap),
                    items[b].py_type(heap)
                )));
                Ordering::Equal
            }
            Err(e) => {
                sort_error = Some(e.into());
                Ordering::Equal
            }
        }
    });

    if let Some(err) = sort_error {
        for item in items {
            item.drop_with_heap(heap);
        }
        return Err(err);
    }

    let mut sorted_items: Vec<Value> = Vec::with_capacity(len);
    for &i in &indices {
        sorted_items.push(std::mem::replace(&mut items[i], Value::Undefined));
    }

    Ok(sorted_items)
}
