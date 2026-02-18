//! Python property descriptor for computed attributes.
//!
//! Properties are descriptors whose value is computed when accessed.
//! When a Property is retrieved via `py_getattr`, its getter is invoked
//! rather than returning the Property itself.

use crate::{args::ArgValues, os::OsFunction, types::CallOutcome};

/// Property descriptor for computed attributes.
///
/// This mirrors Python's descriptor protocol for properties. When accessed,
/// the property's getter is invoked to compute the value.
///
/// # Variants
///
/// Currently only supports OS properties. Future variants:
/// - `Callable(FunctionId)` - user-defined getter functions (@property)
/// - `External(ExtFunctionId)` - external function getters
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum Property {
    /// A property backed by an OS function (e.g., `os.environ`).
    Os(OsFunction),
}

impl Property {
    /// Invokes the property getter, returning the appropriate `CallOutcome`.
    ///
    /// For OS properties, returns `CallOutcome::OsCall` to signal the VM
    /// should yield to the host for the value.
    pub fn get(self) -> CallOutcome {
        match self {
            Self::Os(os_fn) => CallOutcome::OsCall(os_fn, ArgValues::Empty),
        }
    }
}
