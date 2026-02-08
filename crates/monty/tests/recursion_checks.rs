//! Tests for recursion guards and deep nesting protection.
//!
//! These tests verify that deeply nested data structures are handled safely
//! without causing Rust stack overflow. The protection happens at multiple levels:
//!
//! 1. `py_repr_fmt`, `py_eq`, `py_cmp` - Heap recursion guard raises `RecursionError`
//! 2. `MontyObject::from_value_inner` - Returns `<deeply nested>` truncation
//! 3. `dec_ref` - Uses iterative algorithm to avoid stack overflow on drop
//! 4. Serialization - Handles deeply nested `MontyObject` without stack overflow

use monty::{MontyObject, MontyRun};

// === MontyObject::from_value_inner depth limit tests ===

/// Test that deeply nested lists produce truncated MontyObject representation.
///
/// When converting a Value to MontyObject, structures nested beyond the depth
/// limit are replaced with `MontyObject::Repr("<deeply nested>")` to prevent
/// stack overflow during conversion.
#[test]
fn deeply_nested_list_produces_truncated_monty_object() {
    // Create a list nested 100 levels deep (exceeds debug limit of 35)
    let code = r"
x = []
for _ in range(100):
    x = [x]
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    // The result should contain <deeply nested> somewhere in the structure
    let repr = result.py_repr();
    assert!(
        repr.contains("<deeply nested>"),
        "deeply nested list should produce truncated repr, got: {repr}"
    );
}

/// Test that deeply nested tuples produce truncated MontyObject representation.
#[test]
fn deeply_nested_tuple_produces_truncated_monty_object() {
    let code = r"
x: tuple = ()  # type: ignore
for _ in range(100):
    x = (x,)
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    let repr = result.py_repr();
    assert!(
        repr.contains("<deeply nested>"),
        "deeply nested tuple should produce truncated repr, got: {repr}"
    );
}

/// Test that deeply nested dicts produce truncated MontyObject representation.
#[test]
fn deeply_nested_dict_produces_truncated_monty_object() {
    let code = r"
x: dict = {}  # type: ignore
for _ in range(100):
    x = {'a': x}
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    let repr = result.py_repr();
    assert!(
        repr.contains("<deeply nested>"),
        "deeply nested dict should produce truncated repr, got: {repr}"
    );
}

/// Test that deeply nested sets (via frozenset) produce truncated MontyObject representation.
#[test]
fn deeply_nested_frozenset_produces_truncated_monty_object() {
    let code = r"
x: frozenset = frozenset()  # type: ignore
for _ in range(100):
    x = frozenset([x])
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    let repr = result.py_repr();
    assert!(
        repr.contains("<deeply nested>"),
        "deeply nested frozenset should produce truncated repr, got: {repr}"
    );
}

/// Test that moderately nested structures (within limits) work correctly.
///
/// With debug limit of 35, structures nested 20 levels should work fine.
#[test]
fn moderate_nesting_within_limits_works() {
    let code = r"
x = []
for _ in range(20):
    x = [x]
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    let repr = result.py_repr();
    // Should have 21 opening brackets (20 wrappings around [])
    assert_eq!(repr.matches('[').count(), 21, "should have 21 nested levels");
    assert!(
        !repr.contains("<deeply nested>"),
        "moderate nesting should not be truncated, got: {repr}"
    );
}

// === Self-referential structure tests ===

/// Test that self-referential lists still work with cycle detection.
///
/// Cycle detection uses a visited set, not depth counting, so self-referential
/// structures should produce `[...]` placeholder regardless of depth limit.
#[test]
fn self_referential_list_uses_cycle_detection() {
    let code = r"
x = []
x.append(x)
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    // The MontyObject should contain a Cycle variant, not <deeply nested>
    if let MontyObject::List(items) = &result {
        assert_eq!(items.len(), 1);
        assert!(
            matches!(&items[0], MontyObject::Cycle(..)),
            "self-referential list should produce Cycle, got: {:?}",
            items[0]
        );
    } else {
        panic!("expected List, got: {result:?}");
    }

    let repr = result.py_repr();
    assert!(
        repr.contains("[...]"),
        "self-ref list repr should contain [...], got: {repr}"
    );
    assert!(
        !repr.contains("<deeply nested>"),
        "self-ref should use cycle detection, not depth limit"
    );
}

/// Test that mutually referential structures use cycle detection.
#[test]
fn mutually_referential_lists_use_cycle_detection() {
    let code = r"
a = []
b = []
a.append(b)
b.append(a)
a
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    let repr = result.py_repr();
    assert!(
        repr.contains("[...]"),
        "mutually referential lists should use cycle detection, got: {repr}"
    );
}

// === dec_ref iterative algorithm tests ===

/// Test that deeply nested structures don't crash when dropped.
///
/// This verifies that `dec_ref` uses an iterative algorithm. If it were
/// recursive, dropping a structure with 10000 nested levels would overflow
/// the Rust stack.
#[test]
fn deeply_nested_structure_drops_without_stack_overflow() {
    // Create a deeply nested structure, then let it go out of scope
    // If dec_ref is recursive, this would crash
    let code = r"
x = []
for _ in range(10000):
    x = [x]
# x goes out of scope here, dec_ref is called
'done'
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]);

    // The test passes if we don't crash - the deeply nested structure was dropped safely
    assert!(result.is_ok(), "deeply nested structure should drop without crash");
    assert_eq!(result.unwrap(), MontyObject::String("done".to_owned()));
}

/// Test that multiple deeply nested structures can be created and dropped.
#[test]
fn multiple_deeply_nested_structures_drop_safely() {
    let code = r"
for j in range(10):
    x = []
    for i in range(1000):
        x = [x]
    # x goes out of scope each iteration
'done'
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]);

    assert!(result.is_ok(), "multiple deeply nested structures should drop safely");
}

// === JSON serialization tests for deeply nested MontyObject ===

/// Test that deeply nested MontyObject can be serialized to JSON.
///
/// The truncated `<deeply nested>` representation should serialize correctly.
#[test]
fn deeply_nested_monty_object_json_serializable() {
    let code = r"
x = []
for _ in range(100):
    x = [x]
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    // Should serialize without stack overflow
    let json = serde_json::to_string(&result);
    assert!(json.is_ok(), "deeply nested MontyObject should serialize to JSON");

    let json_str = json.unwrap();
    assert!(
        json_str.contains("<deeply nested>"),
        "JSON should contain truncation marker"
    );
}

/// Test that deeply nested MontyObject JSON round-trips correctly.
#[test]
fn deeply_nested_monty_object_json_roundtrip() {
    let code = r"
x = []
for _ in range(100):
    x = [x]
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    let json = serde_json::to_string(&result).unwrap();
    let parsed: MontyObject = serde_json::from_str(&json).unwrap();

    // The parsed result should equal the original
    assert_eq!(result, parsed, "JSON round-trip should preserve equality");
}

/// Test that moderately nested MontyObject serializes without truncation.
#[test]
fn moderate_nesting_json_serializable_without_truncation() {
    let code = r"
x = []
for _ in range(20):
    x = [x]
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    let json = serde_json::to_string(&result).unwrap();
    assert!(
        !json.contains("<deeply nested>"),
        "moderate nesting should not be truncated in JSON"
    );
}

// === Binary serialization tests ===

/// Test that deeply nested MontyObject can be serialized with postcard.
#[test]
fn deeply_nested_monty_object_binary_serializable() {
    let code = r"
x = []
for _ in range(100):
    x = [x]
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    // Should serialize without stack overflow
    let bytes = postcard::to_allocvec(&result);
    assert!(bytes.is_ok(), "deeply nested MontyObject should serialize to binary");
}

/// Test that deeply nested MontyObject binary round-trips correctly.
#[test]
fn deeply_nested_monty_object_binary_roundtrip() {
    let code = r"
x = []
for _ in range(100):
    x = [x]
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    let bytes = postcard::to_allocvec(&result).unwrap();
    let parsed: MontyObject = postcard::from_bytes(&bytes).unwrap();

    assert_eq!(result, parsed, "binary round-trip should preserve equality");
}

// === MontyObject::py_repr tests for deeply nested structures ===

/// Test that MontyObject::py_repr handles deeply nested structures.
///
/// The py_repr method on MontyObject is recursive, but since the underlying
/// Value was already truncated during from_value_inner, it should be safe.
#[test]
fn monty_object_py_repr_handles_deep_nesting() {
    let code = r"
x = []
for _ in range(100):
    x = [x]
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    // py_repr should work without stack overflow because structure was truncated
    let repr = result.py_repr();
    assert!(repr.contains("<deeply nested>"), "py_repr should show truncation");
}

/// Test that moderate nesting in MontyObject::py_repr works correctly.
#[test]
fn monty_object_py_repr_moderate_nesting() {
    let code = r"
x = []
for _ in range(20):
    x = [x]
x
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    let repr = result.py_repr();
    // Should have proper nesting without truncation
    assert!(
        !repr.contains("<deeply nested>"),
        "moderate nesting should not truncate"
    );
    assert_eq!(repr.matches('[').count(), 21, "should show all 21 nesting levels");
}

// === Mixed deep and shallow structure tests ===

/// Test that a structure with both deep and shallow parts is handled correctly.
#[test]
fn mixed_deep_and_shallow_structure() {
    let code = r"
deep = []
for _ in range(100):
    deep = [deep]
shallow = [1, 2, 3]
{'deep': deep, 'shallow': shallow}
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    let repr = result.py_repr();
    // Deep part should be truncated
    assert!(
        repr.contains("<deeply nested>"),
        "deep part should be truncated, got: {repr}"
    );
    // Shallow part should be intact
    assert!(repr.contains("shallow"), "shallow key should be present, got: {repr}");
}

/// Test list containing both deeply nested and immediate values.
#[test]
fn list_with_deep_and_immediate_values() {
    let code = r"
deep = []
for _ in range(100):
    deep = [deep]
[1, deep, 'hello', deep]
";
    let ex = MontyRun::new(code.to_owned(), "test.py", vec![], vec![]).unwrap();
    let result = ex.run_no_limits(vec![]).unwrap();

    let repr = result.py_repr();
    assert!(repr.contains("<deeply nested>"), "deep values should be truncated");
    assert!(repr.contains("'hello'"), "immediate string should be present");

    // Check structure is preserved
    if let MontyObject::List(items) = &result {
        assert_eq!(items.len(), 4, "list should have 4 items");
        assert_eq!(items[0], MontyObject::Int(1));
        assert_eq!(items[2], MontyObject::String("hello".to_owned()));
    } else {
        panic!("expected List");
    }
}
