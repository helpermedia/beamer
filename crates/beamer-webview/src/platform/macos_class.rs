//! Names for the Objective-C classes that are registered at runtime.
//!
//! Class names are global in a process, and a host loads many plugin binaries
//! that each contain their own copy of this code, possibly from different
//! versions of the framework. With a fixed name they would all share the class
//! of the binary that registered it first and run that binary's method
//! implementations on their own objects. The names therefore contain the
//! address of a method implementation in the registering binary, which no
//! other loaded binary has.

use std::ffi::CString;

/// Name for a runtime class of this binary.
///
/// `base` names the class and must differ for every class. `implementation`
/// is the address of one of the functions the class uses as a method
/// implementation, which tells the binaries apart.
pub fn class_name(base: &str, implementation: usize) -> CString {
    CString::new(format!("{base}_{implementation:x}")).expect("class names contain no NUL byte")
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "C" fn first() {}
    extern "C" fn second() {}

    #[test]
    fn test_class_name_contains_the_implementation_address() {
        let name = class_name("BeamerExample", 0x1a2b);
        assert_eq!(name.to_str().unwrap(), "BeamerExample_1a2b");
    }

    #[test]
    fn test_class_names_differ_per_implementation() {
        // Two binaries have their implementations at different addresses, like
        // two functions of one binary.
        assert_ne!(
            class_name("BeamerExample", first as usize),
            class_name("BeamerExample", second as usize)
        );
        assert_eq!(
            class_name("BeamerExample", first as usize),
            class_name("BeamerExample", first as usize)
        );
    }
}
