//! The allocation check with its allocator installed, as a plugin with the
//! `allocation-check` feature has it. A binary of its own, so that the
//! allocator and the global reaction do not reach other tests.

#![cfg(debug_assertions)]

use std::sync::Mutex;

use beamer_core::allocation_check::{self, AllocationCounts, CheckingAllocator, Reaction};
use beamer_core::ProcessScope;

#[global_allocator]
static ALLOCATOR: CheckingAllocator = CheckingAllocator;

/// The reaction is global: the tests take turns.
static TURN: Mutex<()> = Mutex::new(());

fn turn() -> std::sync::MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[test]
fn test_the_check_is_active_with_its_allocator() {
    let _turn = turn();
    drop(vec![1u8]);
    assert!(allocation_check::is_active());
}

#[test]
fn test_allocations_and_frees_in_process_are_counted() {
    let _turn = turn();
    let (_, counts) = allocation_check::count_in_process(|| {
        // Room for one value: the push grows the vector, a reallocation.
        let mut values = vec![1u32];
        values.push(2);
        drop(values);
    });
    assert_eq!(counts, AllocationCounts { allocations: 2, frees: 1 });
}

#[test]
fn test_code_without_allocations_counts_nothing() {
    let _turn = turn();
    let mut values = [0.0f32; 64];
    let (_, counts) = allocation_check::count_in_process(|| {
        for (index, value) in values.iter_mut().enumerate() {
            *value = index as f32 * 0.5;
        }
    });
    assert!(counts.is_empty());
}

#[test]
fn test_a_value_made_outside_and_dropped_inside_is_a_free() {
    let _turn = turn();
    let value = Box::new(5u64);
    let (_, counts) = allocation_check::count_in_process(move || drop(value));
    assert_eq!(counts, AllocationCounts { allocations: 0, frees: 1 });
}

#[test]
fn test_frees_can_be_left_out() {
    let _turn = turn();
    allocation_check::set_check_frees(false);
    let value = Box::new(5u64);
    let (_, counts) = allocation_check::count_in_process(move || drop(value));
    allocation_check::set_check_frees(true);
    assert!(counts.is_empty());
}

#[test]
fn test_permitted_allocations_are_not_counted() {
    let _turn = turn();
    let (_, counts) = allocation_check::count_in_process(|| {
        allocation_check::permit_allocations(|| drop(vec![1u8; 16]));
    });
    assert!(counts.is_empty());
}

#[test]
fn test_allocations_outside_process_are_not_counted() {
    let _turn = turn();
    let (_, counts) = allocation_check::count_in_process(|| {
        // Another thread is not inside process().
        std::thread::scope(|scope| {
            scope.spawn(|| drop(vec![1u8; 16])).join().expect("the thread runs");
        });
    });
    // Spawning the thread allocates on this thread; the vector of the other
    // thread is not counted.
    let (_, spawning) = allocation_check::count_in_process(|| {
        std::thread::scope(|scope| {
            scope.spawn(|| {}).join().expect("the thread runs");
        });
    });
    assert_eq!(counts, spawning);
}

#[test]
fn test_nested_scopes_report_when_the_outermost_ends() {
    let _turn = turn();
    let (_, counts) = allocation_check::count_in_process(|| {
        {
            let _inner = ProcessScope::enter();
            drop(vec![1u8]);
        }
        drop(vec![2u8]);
    });
    assert_eq!(counts, AllocationCounts { allocations: 2, frees: 2 });
}

#[test]
fn test_counting_inside_process_counts_only_its_own() {
    let _turn = turn();
    let (inner, outer) = allocation_check::count_in_process(|| {
        drop(vec![1u8]);
        let (_, inner) = allocation_check::count_in_process(|| drop(vec![2u8]));
        drop(vec![3u8]);
        inner
    });
    assert_eq!(inner, AllocationCounts { allocations: 1, frees: 1 });
    assert_eq!(outer, AllocationCounts { allocations: 2, frees: 2 });
}

#[test]
fn test_logging_keeps_the_plugin_running() {
    let _turn = turn();
    allocation_check::set_reaction(Reaction::Log);
    let scope = ProcessScope::enter();
    drop(vec![1u8]);
    drop(scope);
}

#[test]
fn test_panic_reaction_panics_when_process_ends() {
    let _turn = turn();
    allocation_check::set_reaction(Reaction::Panic);
    let result = std::panic::catch_unwind(|| {
        let _scope = ProcessScope::enter();
        drop(vec![1u8]);
    });
    allocation_check::set_reaction(Reaction::Log);
    let payload = result.expect_err("the end of process() panics");
    let message = payload.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("process() made 1 allocation and 1 free in one call"), "{message}");
    assert!(message.contains("tests/allocation_check.rs"), "{message}");
}

#[test]
fn test_a_panic_in_process_is_not_reported_again() {
    let _turn = turn();
    allocation_check::set_reaction(Reaction::Panic);
    let result = std::panic::catch_unwind(|| {
        let _scope = ProcessScope::enter();
        panic!("the plugin panics");
    });
    allocation_check::set_reaction(Reaction::Log);
    let payload = result.expect_err("the plugin panics");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"the plugin panics"));
}

#[test]
fn test_sending_to_the_web_ui_neither_allocates_nor_frees() {
    let _turn = turn();
    let events = beamer_core::WebViewEventQueue::new();
    events.claim();
    events.editor_opened();
    let mut levels = events.value_sender("levels", [0.0f32; 2]);
    let mut spectrum = events.value_sender("spectrum", vec![0.0f32; 512]);
    let bins = vec![0.5f32; 512];

    let (_, counts) = allocation_check::count_in_process(|| {
        for _ in 0..3 {
            if events.is_gui_open() {
                levels.send(&[0.5, 0.25]);
                spectrum.send(&bins);
                spectrum.send_with(|values| values.fill(0.25));
            }
        }
    });
    assert!(counts.is_empty(), "{counts:?}");
    assert!(events.take_script().is_some());
}
