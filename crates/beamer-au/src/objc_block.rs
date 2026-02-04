//! Minimal helpers for calling Objective-C blocks from Rust.
//!
//! AU hosts provide callbacks as Objective-C blocks (`*const c_void`). A block is an
//! object whose header contains an `invoke` function pointer; the object pointer
//! itself is not callable.
//!
//! This module extracts the `invoke` function pointer so call sites can safely
//! invoke blocks using the correct calling convention (block pointer as first arg).

use std::ffi::c_void;

/// Minimal Objective-C block header layout.
#[repr(C)]
pub(crate) struct ObjCBlockLiteral {
    _isa: *const c_void,
    _flags: u32,
    _reserved: u32,
    invoke: *const c_void,
    _descriptor: *const c_void,
}

/// Extract the block's invoke function pointer.
///
/// # Safety
/// `block` must be a valid Objective-C block object pointer.
#[inline]
pub(crate) unsafe fn invoke_ptr(block: *const c_void) -> *const c_void {
    // SAFETY: Caller guarantees `block` is a valid Objective-C block object.
    // Block layout is stable ABI: isa, flags, reserved, invoke, descriptor.
    // We only read the invoke pointer at the known offset.
    unsafe { (*(block as *const ObjCBlockLiteral)).invoke }
}
