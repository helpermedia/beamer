//! Parameter grouping system for hierarchical organization.
//!
//! This module provides types for organizing parameters into groups (folders)
//! in the DAW's parameter list. Groups form a tree structure that DAWs display
//! hierarchically.
//!
//! # Terminology
//!
//! - **Group**: A named container for parameters (VST3 calls these "Units", AU calls them "Groups")
//! - **Root Group**: The implicit top-level group (ID 0) containing ungrouped parameters
//! - **Nested Group**: A group inside another group
//!
//! # Example
//!
//! ```ignore
//! // Parameter group hierarchy:
//! // Root (id=0)
//! // ├── Filter (id=1)
//! // │   ├── cutoff
//! // │   └── resonance
//! // └── Envelope (id=2)
//! //     ├── attack
//! //     └── release
//! ```

/// Parameter group ID type.
///
/// Groups are used to organize parameters into hierarchical groups in the DAW UI.
/// Each group has a unique ID and can have a parent group.
pub type GroupId = i32;

/// Root group ID constant (parameters with no group).
///
/// The root group (ID 0) always exists and contains ungrouped parameters.
pub const ROOT_GROUP_ID: GroupId = 0;

/// Information about a parameter group.
///
/// Groups form a tree structure via parent_id references:
/// - Root group (id=0, parent=0) always exists implicitly
/// - Top-level groups have parent_id=0
/// - Nested groups reference their parent's group_id
#[derive(Debug, Clone)]
pub struct GroupInfo {
    /// Unique group identifier.
    pub id: GroupId,
    /// Display name shown in DAW (e.g., "Filter", "Amp Envelope").
    pub name: &'static str,
    /// Parent group ID (ROOT_GROUP_ID for top-level groups).
    pub parent_id: GroupId,
}

impl GroupInfo {
    /// Create a new group info.
    pub const fn new(id: GroupId, name: &'static str, parent_id: GroupId) -> Self {
        Self { id, name, parent_id }
    }

    /// Create the root group.
    pub const fn root() -> Self {
        Self {
            id: ROOT_GROUP_ID,
            name: "",
            parent_id: ROOT_GROUP_ID,
        }
    }
}

/// Trait for querying parameter group hierarchy.
///
/// Implemented automatically by `#[derive(Parameters)]` when nested groups are present.
/// Provides information about parameter groups for DAW display.
///
/// Group IDs are assigned dynamically at runtime to support deeply nested groups
/// where the same nested struct type can appear in multiple contexts with
/// different parent groups.
pub trait ParameterGroups {
    /// Total number of groups (including root).
    ///
    /// Returns 1 if there are no groups (just the root group).
    /// For nested groups, this returns 1 + total nested groups (including deeply nested).
    fn group_count(&self) -> usize {
        1 // Default: only root group
    }

    /// Get group info by index.
    ///
    /// Index 0 always returns the root group.
    /// Returns `GroupInfo` by value to support dynamic construction for nested groups.
    fn group_info(&self, index: usize) -> Option<GroupInfo> {
        if index == 0 {
            Some(GroupInfo::root())
        } else {
            None
        }
    }

    /// Find group ID by name (linear search).
    fn find_group_by_name(&self, name: &str) -> Option<GroupId> {
        for i in 0..self.group_count() {
            if let Some(info) = self.group_info(i) {
                if info.name == name {
                    return Some(info.id);
                }
            }
        }
        None
    }
}
