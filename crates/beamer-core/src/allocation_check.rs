//! A check in debug builds that `process()` neither allocates nor frees
//! memory.
//!
//! Allocating or freeing memory on the audio thread can take a lock or wait
//! for the system, so the audio may drop out. On a fast machine that is
//! seldom audible while the code still works, so the mistake is easy to miss:
//! a `Vec` that grows, a `format!`, a `Box`, a `collect()` or dropping the
//! last `Arc` of a value.
//!
//! A plugin turns the check on with the `allocation-check` feature of
//! `beamer`. `#[beamer::export]` then installs a global allocator that
//! counts the allocations and frees made inside a [`ProcessScope`], the time
//! the format wrappers process. Only allocations through the plugin's Rust
//! allocator are seen: memory the host or system libraries allocate is not.
//! In release builds or without the feature nothing is installed; the
//! functions here then do nothing.
//!
//! What happens on a violation is the [`Reaction`]: by default the call
//! that allocated is logged with the stack of the first violation, each
//! place once (see `log_level` in `Config.toml`; with the check on, warnings
//! and errors are logged also without it). A plugin that installs its own
//! global allocator cannot use the feature. One that installs its own logger
//! must do so before the format wrapper starts, or Beamer's logger comes
//! first.
//!
//! ```rust,ignore
//! // In a test of the plugin: the processor neither allocates nor frees.
//! let (_, counts) = beamer::allocation_check::count_in_process(|| {
//!     processor.process(&mut buffer, &mut aux, &context);
//! });
//! if beamer::allocation_check::is_active() {
//!     assert!(counts.is_empty(), "{counts:?}");
//! }
//! ```
//!
//! [`ProcessScope`]: crate::ProcessScope

/// What the check does when `process()` allocates or frees memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    /// Log the call with the stack of its first violation, each place once.
    /// The plugin keeps running.
    Log,
    /// Log as [`Log`](Reaction::Log), then panic when the call returns. For
    /// tests: in a host an Audio Unit stops rendering, a VST3 host aborts.
    Panic,
    /// Abort the process at the allocation or free, so that a debugger
    /// stops with the stack of the audio thread.
    Abort,
}

/// The allocations and frees counted inside `process()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocationCounts {
    /// Allocations, a growing reallocation included.
    pub allocations: u32,
    /// Frees of memory.
    pub frees: u32,
}

impl AllocationCounts {
    /// Whether nothing was allocated or freed.
    pub fn is_empty(&self) -> bool {
        self.allocations == 0 && self.frees == 0
    }

    #[cfg(all(feature = "allocation-check", debug_assertions))]
    fn add(self, other: Self) -> Self {
        Self {
            allocations: self.allocations.saturating_add(other.allocations),
            frees: self.frees.saturating_add(other.frees),
        }
    }
}

/// Whether the check runs: the feature is on in a debug build and the
/// plugin's allocator is the checking one. A test that expects a count
/// skips its assertion when this is false.
pub fn is_active() -> bool {
    imp::is_active()
}

/// Set what the check does on a violation, for all threads. Default:
/// [`Reaction::Log`].
pub fn set_reaction(reaction: Reaction) {
    imp::set_reaction(reaction);
}

/// Whether frees count as violations, for all threads. Default: true.
///
/// A value made on another thread and dropped in `process()` frees its
/// memory there. Turn this off to look at allocations alone.
pub fn set_check_frees(check: bool) {
    imp::set_check_frees(check);
}

/// Run `f` without the check, for code in `process()` that may allocate on
/// purpose (a measurement in a debug build, a log line). Also inside the
/// call of [`count_in_process`].
pub fn permit_allocations<R>(f: impl FnOnce() -> R) -> R {
    imp::permit_allocations(f)
}

/// Run `f` as `process()` runs (inside a [`ProcessScope`]) and count its
/// allocations and frees on this thread, without the reaction. For tests:
/// the counts are 0 when the check is not active (see [`is_active`]).
///
/// [`ProcessScope`]: crate::ProcessScope
pub fn count_in_process<R>(f: impl FnOnce() -> R) -> (R, AllocationCounts) {
    imp::count_in_process(f)
}

/// The end of the outermost [`ProcessScope`](crate::ProcessScope) on this
/// thread: react to what the scope counted.
#[cfg(debug_assertions)]
#[inline]
pub(crate) fn process_ended() {
    imp::process_ended();
}

#[cfg(all(feature = "allocation-check", debug_assertions))]
pub use imp::CheckingAllocator;

#[cfg(all(feature = "allocation-check", debug_assertions))]
mod imp {
    use super::{AllocationCounts, Reaction};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::Mutex;

    /// Frames kept of the stack of a first violation.
    const MAX_FRAMES: usize = 48;

    /// Frames of a stack shown in a report, after the check's own. The
    /// system log cuts long messages.
    const SHOWN_FRAMES: usize = 6;

    /// Frames from the top of a stack that tell its place: the host's part
    /// below the process call may differ from call to call.
    const PLACE_FRAMES: usize = 24;

    /// Places reported before the check stops reporting new ones.
    const MAX_REPORTED_PLACES: usize = 256;

    /// The return addresses of a stack, captured without allocating.
    #[derive(Clone, Copy)]
    struct Frames {
        addresses: [usize; MAX_FRAMES],
        count: usize,
    }

    impl Frames {
        const EMPTY: Self = Self { addresses: [0; MAX_FRAMES], count: 0 };

        fn addresses(&self) -> &[usize] {
            &self.addresses[..self.count]
        }
    }

    /// What a violation did.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Kind {
        Allocation,
        Free,
    }

    // Constant thread-locals without a destructor: reading them from the
    // allocator never allocates.
    thread_local! {
        /// Depth of `permit_allocations` calls on this thread.
        static PERMITTED: Cell<u32> = const { Cell::new(0) };
        /// Whether this thread is noting a violation now: what that does
        /// itself is not checked.
        static NOTING: Cell<bool> = const { Cell::new(false) };
        /// What the current process call counted.
        static COUNTS: Cell<AllocationCounts> = const {
            Cell::new(AllocationCounts { allocations: 0, frees: 0 })
        };
        /// The kind and stack of the first violation of the current call.
        static FIRST: Cell<(Kind, Frames)> = const { Cell::new((Kind::Allocation, Frames::EMPTY)) };
        /// Inside `count_in_process`: the counts so far, without reaction.
        static COUNTING: Cell<Option<AllocationCounts>> = const { Cell::new(None) };
    }

    static REACTION: AtomicU8 = AtomicU8::new(0);
    static CHECK_FREES: AtomicBool = AtomicBool::new(true);

    /// Whether the checking allocator handled an allocation, so it is the
    /// global allocator.
    static INSTALLED: AtomicBool = AtomicBool::new(false);

    /// Hashes of the stacks reported, so that each place is reported once.
    static REPORTED: Mutex<Vec<u64>> = Mutex::new(Vec::new());

    pub(super) fn is_active() -> bool {
        INSTALLED.load(Ordering::Relaxed)
    }

    pub(super) fn set_reaction(reaction: Reaction) {
        let value = match reaction {
            Reaction::Log => 0,
            Reaction::Panic => 1,
            Reaction::Abort => 2,
        };
        REACTION.store(value, Ordering::Relaxed);
    }

    fn reaction() -> Reaction {
        match REACTION.load(Ordering::Relaxed) {
            1 => Reaction::Panic,
            2 => Reaction::Abort,
            _ => Reaction::Log,
        }
    }

    pub(super) fn set_check_frees(check: bool) {
        CHECK_FREES.store(check, Ordering::Relaxed);
    }

    pub(super) fn permit_allocations<R>(f: impl FnOnce() -> R) -> R {
        /// Ends the permission, also when `f` panics.
        struct Permission;
        impl Drop for Permission {
            fn drop(&mut self) {
                PERMITTED.with(|depth| depth.set(depth.get().saturating_sub(1)));
            }
        }
        PERMITTED.with(|depth| depth.set(depth.get() + 1));
        let _permission = Permission;
        f()
    }

    pub(super) fn count_in_process<R>(f: impl FnOnce() -> R) -> (R, AllocationCounts) {
        /// The counts of an outer process call and the counting of an outer
        /// `count_in_process`, restored also when `f` panics.
        struct Outer {
            counts: AllocationCounts,
            counting: Option<AllocationCounts>,
        }
        impl Drop for Outer {
            fn drop(&mut self) {
                COUNTS.with(|counts| counts.set(self.counts));
                COUNTING.with(|counting| counting.set(self.counting));
            }
        }
        let outer = Outer {
            counts: COUNTS.with(|counts| counts.replace(AllocationCounts::default())),
            counting: COUNTING.with(|counting| counting.replace(Some(AllocationCounts::default()))),
        };
        let result = {
            let _scope = crate::ProcessScope::enter();
            f()
        };
        // The scope handed its counts over when it was the outermost one;
        // inside another process call they are still in COUNTS.
        let counted = COUNTING.with(Cell::get).unwrap_or_default();
        let counts = counted.add(COUNTS.with(|counts| counts.get()));
        drop(outer);
        (result, counts)
    }

    /// Note an allocation or a free on this thread, from the allocator.
    /// Allocates nothing.
    #[inline]
    fn note(kind: Kind) {
        let checked = crate::audio_thread::in_process()
            && PERMITTED.try_with(Cell::get) == Ok(0)
            && NOTING.try_with(Cell::get) == Ok(false)
            // Panicking allocates: a plugin that panics is not reported
            // for that as well.
            && !std::thread::panicking();
        if !checked {
            return;
        }
        NOTING.with(|noting| noting.set(true));
        let counts = COUNTS.with(|counts| {
            let mut value = counts.get();
            match kind {
                Kind::Allocation => value.allocations = value.allocations.saturating_add(1),
                Kind::Free => value.frees = value.frees.saturating_add(1),
            }
            counts.set(value);
            value
        });
        if counts.allocations + counts.frees == 1 && COUNTING.with(Cell::get).is_none() {
            FIRST.with(|first| first.set((kind, capture())));
        }
        if reaction() == Reaction::Abort && COUNTING.with(Cell::get).is_none() {
            // A failed write must not unwind out of the allocator.
            let _ = std::io::Write::write_all(
                &mut std::io::stderr(),
                b"Beamer allocation check: process() allocated or freed memory, aborting\n",
            );
            std::process::abort();
        }
        NOTING.with(|noting| noting.set(false));
    }

    /// The return addresses of the current stack. Walking it allocates
    /// nothing; the names are looked up for the report.
    fn capture() -> Frames {
        let mut frames = Frames::EMPTY;
        backtrace::trace(|frame| {
            frames.addresses[frames.count] = frame.ip() as usize;
            frames.count += 1;
            frames.count < MAX_FRAMES
        });
        frames
    }

    pub(super) fn process_ended() {
        let counts = COUNTS.with(|counts| counts.replace(AllocationCounts::default()));
        if counts.is_empty() {
            return;
        }
        if let Some(total) = COUNTING.with(Cell::get) {
            COUNTING.with(|counting| counting.set(Some(total.add(counts))));
            return;
        }
        let (kind, frames) = FIRST.with(|first| first.replace((Kind::Allocation, Frames::EMPTY)));
        // The report allocates; outside the scope that is not checked.
        let Some(message) = report(counts, kind, &frames) else {
            return;
        };
        log::error!("{message}");
        // A panic while the thread unwinds already would abort.
        if reaction() == Reaction::Panic && !std::thread::panicking() {
            panic!("{message}");
        }
    }

    /// The report of a process call, `None` when its place was reported
    /// before (always reported with `Reaction::Panic`).
    fn report(counts: AllocationCounts, kind: Kind, frames: &Frames) -> Option<String> {
        use std::fmt::Write as _;
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        frames.addresses().iter().take(PLACE_FRAMES).for_each(|address| address.hash(&mut hasher));
        let place = hasher.finish();
        {
            let mut reported = REPORTED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let known = reported.contains(&place);
            let full = reported.len() >= MAX_REPORTED_PLACES;
            if !known && !full {
                reported.push(place);
            }
            if (known || full) && reaction() != Reaction::Panic {
                return None;
            }
        }

        let what = match kind {
            Kind::Allocation => "allocation",
            Kind::Free => "free",
        };
        let mut message = format!(
            "process() made {} and {} in one call. On the audio thread that can make \
             the audio drop out. The first {what} was at:",
            amount(counts.allocations, "allocation"),
            amount(counts.frees, "free")
        );
        for line in frame_lines(frames) {
            let _ = write!(message, "\n    {line}");
        }
        if reaction() == Reaction::Log {
            message.push_str("\nFurther calls from this place are not reported.");
        }
        Some(message)
    }

    /// `count` of a `thing`: "1 free", "2 frees".
    fn amount(count: u32, thing: &str) -> String {
        if count == 1 {
            format!("1 {thing}")
        } else {
            format!("{count} {thing}s")
        }
    }

    /// The names (and places, when known) of the frames of a stack, from the
    /// first one outside the check and the standard library: the code that
    /// allocated.
    fn frame_lines(frames: &Frames) -> Vec<String> {
        let mut lines = Vec::new();
        let mut in_check = true;
        for &address in frames.addresses() {
            backtrace::resolve(address as *mut std::ffi::c_void, |symbol| {
                let name = symbol.name().map(|name| format!("{name:#}")).unwrap_or_default();
                // The path of a function, or of the trait it implements:
                // `<u8 as alloc::vec::SpecFromElem>::from_elem` is in alloc.
                let path = match name.split_once(" as ") {
                    Some((_, implemented)) if name.starts_with('<') => implemented,
                    _ => name.trim_start_matches('<'),
                };
                let checking = path.starts_with("backtrace::")
                    || path.starts_with("beamer_core::allocation_check::")
                    || path.starts_with("alloc::")
                    || path.starts_with("core::")
                    || path.starts_with("std::")
                    || path.starts_with("__rust")
                    || path.starts_with("__rdl")
                    || name.is_empty();
                if in_check && checking {
                    return;
                }
                in_check = false;
                if name.is_empty() || lines.len() >= SHOWN_FRAMES {
                    return;
                }
                match (symbol.filename(), symbol.lineno()) {
                    (Some(file), Some(line)) => lines.push(format!("{name} ({}:{line})", short_path(file))),
                    _ => lines.push(name),
                }
            });
        }
        if lines.is_empty() {
            lines.push("(no symbols: build with debug info to see the stack)".to_string());
        }
        lines
    }

    /// The last three parts of a source path: enough to find the file.
    fn short_path(file: &std::path::Path) -> String {
        let parts: Vec<_> = file.iter().collect();
        let start = parts.len().saturating_sub(3);
        parts[start..].iter().map(|part| part.to_string_lossy()).collect::<Vec<_>>().join("/")
    }

    /// The global allocator of a plugin with the allocation check: the
    /// system allocator, counting what `process()` allocates and frees.
    ///
    /// `#[beamer::export]` installs it with the `allocation-check` feature
    /// of `beamer` in debug builds.
    pub struct CheckingAllocator;

    // SAFETY: Every call is passed on to the system allocator unchanged.
    // `note` only reads and writes thread-locals and atomics (the stack
    // walk does not allocate) and never unwinds.
    unsafe impl GlobalAlloc for CheckingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            INSTALLED.store(true, Ordering::Relaxed);
            note(Kind::Allocation);
            // SAFETY: The caller's guarantees are passed on.
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            INSTALLED.store(true, Ordering::Relaxed);
            note(Kind::Allocation);
            // SAFETY: The caller's guarantees are passed on.
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            note(Kind::Allocation);
            // SAFETY: The caller's guarantees are passed on.
            unsafe { System.realloc(ptr, layout, new_size) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            if CHECK_FREES.load(Ordering::Relaxed) {
                note(Kind::Free);
            }
            // SAFETY: The caller's guarantees are passed on.
            unsafe { System.dealloc(ptr, layout) }
        }
    }
}

#[cfg(not(all(feature = "allocation-check", debug_assertions)))]
mod imp {
    use super::{AllocationCounts, Reaction};

    pub(super) fn is_active() -> bool {
        false
    }

    pub(super) fn set_reaction(_reaction: Reaction) {}

    pub(super) fn set_check_frees(_check: bool) {}

    pub(super) fn permit_allocations<R>(f: impl FnOnce() -> R) -> R {
        f()
    }

    pub(super) fn count_in_process<R>(f: impl FnOnce() -> R) -> (R, AllocationCounts) {
        let _scope = crate::ProcessScope::enter();
        (f(), AllocationCounts::default())
    }

    #[cfg(debug_assertions)]
    #[inline]
    pub(super) fn process_ended() {}
}
