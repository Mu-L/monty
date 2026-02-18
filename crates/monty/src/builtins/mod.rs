//! Python builtin functions, types, and exception constructors.
//!
//! This module provides the interpreter-native implementation of Python builtins.
//! Each builtin function has its own submodule for organization.

mod abs;
mod all;
mod any;
mod bin;
mod chr;
mod divmod;
mod enumerate;
mod hash;
mod hex;
mod id;
mod isinstance;
mod len;
mod map;
mod min_max; // min and max share implementation
mod next;
mod oct;
mod ord;
mod pow;
mod print;
mod repr;
mod reversed;
mod round;
mod sorted;
mod sum;
mod type_;
mod zip;

use std::{fmt::Write, str::FromStr};

use strum::{Display, EnumString, FromRepr, IntoStaticStr};

use crate::{
    args::ArgValues,
    bytecode::VM,
    exception_private::{ExcType, RunResult},
    heap::Heap,
    intern::Interns,
    io::PrintWriter,
    resource::ResourceTracker,
    types::{CallOutcome, Type},
    value::Value,
};

/// Enumerates every interpreter-native Python builtins
///
/// Uses strum derives for automatic `Display`, `FromStr`, and `AsRef<str>` implementations.
/// All variants serialize to lowercase (e.g., `Print` -> "print").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum Builtins {
    /// A builtin function like `print`, `len`, `type`, etc.
    Function(BuiltinsFunctions),
    /// An exception type constructor like `ValueError`, `TypeError`, etc.
    ExcType(ExcType),
    /// A type constructor like `list`, `dict`, `int`, etc.
    Type(Type),
}

impl Builtins {
    /// Calls this builtin with full VM context, returning a `CallOutcome`.
    ///
    /// Builtin functions that need VM access (e.g., `sorted()` with key functions)
    /// are dispatched through `BuiltinsFunctions::call`. All others delegate to
    /// `call_basic` which only needs heap/interns/print_writer.
    ///
    /// Exception types and type constructors always complete synchronously.
    pub fn call<T: ResourceTracker>(self, vm: &mut VM<'_, '_, T>, args: ArgValues) -> RunResult<CallOutcome> {
        match self {
            Self::Function(b) => b.call(vm, args),
            Self::ExcType(exc) => exc.call(vm.heap, args, vm.interns).map(CallOutcome::Value),
            Self::Type(t) => t.call(vm.heap, args, vm.interns).map(CallOutcome::Value),
        }
    }

    /// Calls this builtin without VM context.
    ///
    /// Used by `map()` and other contexts where builtins are called as function
    /// arguments without needing VM access.
    pub fn call_basic(
        self,
        heap: &mut Heap<impl ResourceTracker>,
        args: ArgValues,
        interns: &Interns,
        print_writer: &mut PrintWriter<'_>,
    ) -> RunResult<Value> {
        match self {
            Self::Function(b) => b.call_basic(heap, args, interns, print_writer),
            Self::ExcType(exc) => exc.call(heap, args, interns),
            Self::Type(t) => t.call(heap, args, interns),
        }
    }

    /// Writes the Python repr() string for this callable to a formatter.
    pub fn py_repr_fmt<W: Write>(self, f: &mut W) -> std::fmt::Result {
        match self {
            Self::Function(b) => write!(f, "<built-in function {b}>"),
            Self::ExcType(e) => write!(f, "<class '{e}'>"),
            Self::Type(t) => write!(f, "<class '{t}'>"),
        }
    }

    /// Returns the type of this builtin.
    pub fn py_type(self) -> Type {
        match self {
            Self::Function(_) => Type::BuiltinFunction,
            Self::ExcType(_) => Type::Type,
            Self::Type(_) => Type::Type,
        }
    }
}

impl FromStr for Builtins {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Priority: BuiltinsFunctions > ExcType > Type
        // Only matches names that are true Python builtins (accessible without imports).
        if let Ok(b) = BuiltinsFunctions::from_str(s) {
            Ok(Self::Function(b))
        } else if let Ok(exc) = ExcType::from_str(s) {
            Ok(Self::ExcType(exc))
        } else if let Some(t) = Type::from_builtin_name(s) {
            Ok(Self::Type(t))
        } else {
            Err(())
        }
    }
}

/// Enumerates every interpreter-native Python builtin function.
///
/// Listed alphabetically per https://docs.python.org/3/library/functions.html
/// Commented-out variants are not yet implemented.
///
/// Note: Type constructors are handled by the `Type` enum, not here.
///
/// Uses strum derives for automatic `Display`, `FromStr`, and `IntoStaticStr` implementations.
/// All variants serialize to lowercase (e.g., `Print` -> "print").
#[derive(
    Debug,
    Clone,
    Copy,
    Display,
    EnumString,
    FromRepr,
    IntoStaticStr,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[strum(serialize_all = "lowercase")]
#[repr(u8)]
pub enum BuiltinsFunctions {
    Abs,
    // Aiter,
    All,
    // Anext,
    Any,
    // Ascii,
    Bin,
    // bool - handled by Type enum
    // Breakpoint,
    // bytearray - handled by Type enum
    // bytes - handled by Type enum
    // Callable,
    Chr,
    // Classmethod,
    // Compile,
    // complex - handled by Type enum
    // Delattr,
    // dict - handled by Type enum
    // Dir,
    Divmod,
    Enumerate,
    // Eval,
    // Exec,
    // Filter,
    // float - handled by Type enum
    // Format,
    // frozenset - handled by Type enum
    // Getattr,
    // Globals,
    // Hasattr,
    Hash,
    // Help,
    Hex,
    Id,
    // Input,
    // int - handled by Type enum
    Isinstance,
    // Issubclass,
    // Iter - handled by Type enum
    Len,
    // list - handled by Type enum
    // Locals,
    Map,
    Max,
    // memoryview - handled by Type enum
    Min,
    Next,
    // object - handled by Type enum
    Oct,
    // Open,
    Ord,
    Pow,
    Print,
    // Property,
    // range - handled by Type enum
    Repr,
    Reversed,
    Round,
    // set - handled by Type enum
    // Setattr,
    // Slice,
    Sorted,
    // Staticmethod,
    // str - handled by Type enum
    Sum,
    // Super,
    // tuple - handled by Type enum
    Type,
    // Vars,
    Zip,
    // __import__ - not planned
}

impl BuiltinsFunctions {
    /// Executes the builtin with full VM context.
    ///
    /// Builtins that need VM access (e.g., `sorted()` for calling user-defined key
    /// functions via `call_sync`) are dispatched directly here. All others delegate
    /// to `call_basic` which only needs heap/interns/print_writer, then wrap the
    /// result in `CallOutcome::Value`.
    pub(crate) fn call<T: ResourceTracker>(self, vm: &mut VM<'_, '_, T>, args: ArgValues) -> RunResult<CallOutcome> {
        match self {
            // Sorted needs VM access for call_sync (user-defined key functions)
            Self::Sorted => sorted::builtin_sorted(vm, args),
            // All other builtins delegate to call_basic
            _ => self
                .call_basic(vm.heap, args, vm.interns, vm.print_writer)
                .map(CallOutcome::Value),
        }
    }

    /// Executes the builtin without VM context.
    ///
    /// Used by `call_key_function` in `list.sort()` where no VM is available,
    /// and as the default path for most builtins that don't need VM access.
    pub(crate) fn call_basic(
        self,
        heap: &mut Heap<impl ResourceTracker>,
        args: ArgValues,
        interns: &Interns,
        print_writer: &mut PrintWriter<'_>,
    ) -> RunResult<Value> {
        match self {
            Self::Abs => abs::builtin_abs(heap, args),
            Self::All => all::builtin_all(heap, args, interns),
            Self::Any => any::builtin_any(heap, args, interns),
            Self::Bin => bin::builtin_bin(heap, args),
            Self::Chr => chr::builtin_chr(heap, args),
            Self::Divmod => divmod::builtin_divmod(heap, args),
            Self::Enumerate => enumerate::builtin_enumerate(heap, args, interns),
            Self::Hash => hash::builtin_hash(heap, args, interns),
            Self::Hex => hex::builtin_hex(heap, args),
            Self::Id => id::builtin_id(heap, args),
            Self::Isinstance => isinstance::builtin_isinstance(heap, args),
            Self::Len => len::builtin_len(heap, args, interns),
            Self::Map => map::builtin_map(heap, args, interns, print_writer),
            Self::Max => min_max::builtin_max(heap, args, interns),
            Self::Min => min_max::builtin_min(heap, args, interns),
            Self::Next => next::builtin_next(heap, args, interns),
            Self::Oct => oct::builtin_oct(heap, args),
            Self::Ord => ord::builtin_ord(heap, args, interns),
            Self::Pow => pow::builtin_pow(heap, args),
            Self::Print => print::builtin_print(heap, args, interns, print_writer),
            Self::Repr => repr::builtin_repr(heap, args, interns),
            Self::Reversed => reversed::builtin_reversed(heap, args, interns),
            Self::Round => round::builtin_round(heap, args),
            Self::Sorted => {
                // sorted() with no kwargs — call_basic is used from list.sort's call_key_function
                sorted::builtin_sorted_basic(heap, args, interns)
            }
            Self::Sum => sum::builtin_sum(heap, args, interns),
            Self::Type => type_::builtin_type(heap, args),
            Self::Zip => zip::builtin_zip(heap, args, interns),
        }
    }
}
