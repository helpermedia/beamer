//! Ivar wrappers for objc2 instance variables.
//!
//! This module provides two wrappers for storing values in objc2 ivars while
//! satisfying the `Clone + Default` requirements:
//!
//! - [`IvarArc<T>`]: For Arc-wrapped types (complex objects, shared state)
//! - [`IvarCell<T>`]: For primitive/copyable types (f64, u32, etc.)
//!
//! # Why These Exist
//!
//! objc2's `define_class!` macro requires ivars to implement `Clone + Default`.
//! These wrappers provide:
//!
//! - `Default` via zero initialization
//! - Controlled initialization and cleanup
//! - Safe interior mutability via `UnsafeCell`
//! - Proper `Drop` implementations to prevent leaks
//!
//! # Safety
//!
//! Both types use `UnsafeCell` for interior mutability. The AU framework guarantees
//! that property access is synchronized, so we can safely mutate from `&self`.

use std::cell::UnsafeCell;
use std::mem::ManuallyDrop;
use std::sync::Arc;

/// Wrapper for `Arc<T>` that can be stored in objc2 ivars.
///
/// Implements `Clone + Default` as required by objc2, while providing
/// controlled initialization and cleanup.
///
/// # Example
///
/// ```ignore
/// use beamer_au::ivar_arc::IvarArc;
/// use std::sync::{Arc, Mutex};
///
/// #[derive(Clone, Default)]
/// struct MyIvars {
///     plugin: IvarArc<Mutex<MyPlugin>>,
/// }
///
/// // During init:
/// unsafe { ivars.plugin.init(Arc::new(Mutex::new(MyPlugin::new()))); }
///
/// // During use:
/// if let Some(arc) = unsafe { ivars.plugin.get() } {
///     let guard = arc.lock().unwrap();
///     // use plugin...
/// }
///
/// // During dealloc:
/// unsafe { ivars.plugin.clear(); }
/// ```
pub struct IvarArc<T: ?Sized> {
    /// Inner storage using UnsafeCell for interior mutability.
    /// ManuallyDrop ensures we control when Arc is dropped.
    inner: UnsafeCell<ManuallyDrop<Option<Arc<T>>>>,
}

impl<T: ?Sized> IvarArc<T> {
    /// Create a new, uninitialized IvarArc.
    ///
    /// The Arc is `None` until `init()` is called.
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(ManuallyDrop::new(None)),
        }
    }

    /// Initialize with an Arc.
    ///
    /// Should be called once during AU initialization. Subsequent calls
    /// will overwrite the previous Arc (dropping it).
    ///
    /// # Safety
    ///
    /// Caller must ensure no concurrent access during initialization.
    /// This is guaranteed by AU's initialization lifecycle.
    pub unsafe fn init(&self, value: Arc<T>) {
        let inner = &mut *self.inner.get();
        // Drop previous value if any
        ManuallyDrop::drop(inner);
        *inner = ManuallyDrop::new(Some(value));
    }

    /// Get a clone of the Arc.
    ///
    /// Returns `None` if `init()` hasn't been called.
    ///
    /// # Safety
    ///
    /// Caller must ensure `init()` was called before this.
    /// The returned Arc is a clone (reference count incremented),
    /// so it's safe to hold across calls.
    pub unsafe fn get(&self) -> Option<Arc<T>> {
        let inner = &*self.inner.get();
        (**inner).as_ref().map(Arc::clone)
    }

    /// Check if initialized.
    ///
    /// # Safety
    ///
    /// Caller must ensure no concurrent mutation.
    pub unsafe fn is_initialized(&self) -> bool {
        let inner = &*self.inner.get();
        (**inner).is_some()
    }

    /// Clear the Arc, dropping it.
    ///
    /// Should be called during AU deallocation to prevent leaks.
    ///
    /// # Safety
    ///
    /// Caller must ensure no concurrent access and that the Arc
    /// is not being used elsewhere in a way that would cause UB.
    pub unsafe fn clear(&self) {
        let inner = &mut *self.inner.get();
        ManuallyDrop::drop(inner);
        *inner = ManuallyDrop::new(None);
    }
}

impl<T: ?Sized> Default for IvarArc<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized> Clone for IvarArc<T> {
    /// Clone creates a new IvarArc with a clone of the inner Arc.
    ///
    /// # Safety Note
    ///
    /// This clone operation accesses the inner Arc without synchronization.
    /// In practice, objc2 clones ivars during object allocation before
    /// any methods are called, so this is safe.
    fn clone(&self) -> Self {
        let inner = unsafe { &*self.inner.get() };
        Self {
            inner: UnsafeCell::new(ManuallyDrop::new((**inner).clone())),
        }
    }
}

// SAFETY: Arc<T> is Send/Sync when T is, and we only access
// through methods that respect AU's synchronization guarantees.
unsafe impl<T: ?Sized + Send + Sync> Send for IvarArc<T> {}
unsafe impl<T: ?Sized + Send + Sync> Sync for IvarArc<T> {}

impl<T: ?Sized> Drop for IvarArc<T> {
    /// Drop the inner Arc if it was initialized.
    ///
    /// This ensures proper cleanup even if `clear()` wasn't called explicitly
    /// (e.g., abnormal termination or host crash).
    fn drop(&mut self) {
        // SAFETY: We have &mut self, so exclusive access is guaranteed.
        // ManuallyDrop::drop is safe to call once.
        unsafe {
            ManuallyDrop::drop(&mut *self.inner.get());
        }
    }
}

// =============================================================================
// IvarCell - For primitive/copyable types
// =============================================================================

/// Wrapper for values that need Clone+Default for objc2 ivars with UnsafeCell access.
///
/// Similar to `IvarArc<T>` but for primitive types that don't need Arc wrapping.
/// Provides direct mutable access via UnsafeCell.
///
/// # Use Cases
///
/// - Storing primitive types (f64, u32, etc.) in ivars
/// - Storing types that implement Default + Clone
/// - When you need direct `&mut T` access instead of Arc cloning
///
/// # Example
///
/// ```ignore
/// use beamer_au::ivar_arc::IvarCell;
///
/// #[derive(Clone, Default)]
/// struct MyIvars {
///     sample_rate: IvarCell<f64>,
///     max_frames: IvarCell<u32>,
/// }
///
/// // During use:
/// unsafe {
///     *ivars.sample_rate.get() = 48000.0;
///     let sr = *ivars.sample_rate.get();
/// }
/// ```
#[derive(Default)]
pub struct IvarCell<T: Default> {
    inner: UnsafeCell<T>,
}

impl<T: Default + Clone> Clone for IvarCell<T> {
    fn clone(&self) -> Self {
        Self {
            inner: UnsafeCell::new(unsafe { (*self.inner.get()).clone() }),
        }
    }
}

impl<T: Default> IvarCell<T> {
    /// Create a new IvarCell with default value.
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(unsafe {
                // SAFETY: This is only safe for types where Default::default()
                // is a const fn or can be zero-initialized. For most primitives
                // this works. For complex types, use Default trait impl instead.
                std::mem::zeroed()
            }),
        }
    }

    /// Get a mutable pointer to the inner value.
    ///
    /// # Safety
    ///
    /// The caller must ensure no concurrent access and that the value
    /// is not mutated while any references exist.
    #[inline]
    pub fn get(&self) -> *mut T {
        self.inner.get()
    }

    /// Set the value.
    ///
    /// # Safety
    ///
    /// The caller must ensure no concurrent access.
    #[inline]
    pub unsafe fn set(&self, value: T) {
        *self.inner.get() = value;
    }

    /// Get a copy of the value (for Copy types).
    ///
    /// # Safety
    ///
    /// The caller must ensure no concurrent mutation.
    #[inline]
    pub unsafe fn read(&self) -> T
    where
        T: Copy,
    {
        *self.inner.get()
    }
}

// SAFETY: Access is synchronized by AU framework guarantees.
// The AU framework ensures that methods are called from a single thread
// or with proper synchronization.
unsafe impl<T: Default> Send for IvarCell<T> {}
unsafe impl<T: Default> Sync for IvarCell<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn test_ivar_arc_init_and_get() {
        let ivar: IvarArc<Mutex<i32>> = IvarArc::new();

        unsafe {
            assert!(!ivar.is_initialized());

            ivar.init(Arc::new(Mutex::new(42)));
            assert!(ivar.is_initialized());

            let arc = ivar.get().unwrap();
            assert_eq!(*arc.lock().unwrap(), 42);

            ivar.clear();
            assert!(!ivar.is_initialized());
        }
    }

    #[test]
    fn test_ivar_arc_clone() {
        let ivar1: IvarArc<Mutex<i32>> = IvarArc::new();

        unsafe {
            ivar1.init(Arc::new(Mutex::new(42)));

            let ivar2 = ivar1.clone();
            let arc1 = ivar1.get().unwrap();
            let arc2 = ivar2.get().unwrap();

            // Both should point to same data
            *arc1.lock().unwrap() = 100;
            assert_eq!(*arc2.lock().unwrap(), 100);
        }
    }

    #[test]
    fn test_ivar_arc_default() {
        let ivar: IvarArc<Mutex<i32>> = IvarArc::default();
        unsafe {
            assert!(!ivar.is_initialized());
            assert!(ivar.get().is_none());
        }
    }

    #[test]
    fn test_ivar_cell_basic() {
        let cell: IvarCell<f64> = IvarCell::default();
        unsafe {
            *cell.get() = 48000.0;
            assert_eq!(*cell.get(), 48000.0);
        }
    }

    #[test]
    fn test_ivar_cell_set_and_read() {
        let cell: IvarCell<u32> = IvarCell::new();
        unsafe {
            cell.set(1024);
            assert_eq!(cell.read(), 1024);
        }
    }

    #[test]
    fn test_ivar_cell_clone() {
        let cell1: IvarCell<i32> = IvarCell::default();
        unsafe {
            *cell1.get() = 42;
        }

        let cell2 = cell1.clone();
        unsafe {
            // Clone creates independent copy
            assert_eq!(*cell2.get(), 42);
            *cell2.get() = 100;
            assert_eq!(*cell1.get(), 42); // Original unchanged
        }
    }
}
