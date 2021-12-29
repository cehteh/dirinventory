use std::sync::Arc;
use std::fmt::{Debug, Formatter, Result};

use crate::*;

/// Messages on the input queue, directories to be processed.
#[derive(Debug)]
pub enum DirectoryGatherMessage {
    /// Path and parent handle of a directory to be traversed. The handle to the directory
    /// itself will be opened by the thread processing it.
    TraverseDirectory {
        /// The path to the Object
        path:       Arc<ObjectPath>,
        /// Optional handle to the parent directory
        parent_dir: Option<Arc<openat::Dir>>,
    },
    // internally used by drop to terminate all threads
    // Shutdown,
}

impl DirectoryGatherMessage {
    /// Create a new 'TraverseDirectory' message.
    pub fn new_dir(path: Arc<ObjectPath>) -> Self {
        DirectoryGatherMessage::TraverseDirectory {
            path,
            parent_dir: None,
        }
    }

    /// Attach a parent handle to a 'TraverseDirectory' message. Must not be used with other messages!
    #[must_use]
    pub fn with_parent(mut self, parent: Arc<openat::Dir>) -> Self {
        debug_assert!(matches!(
            self,
            DirectoryGatherMessage::TraverseDirectory { .. }
        ));
        let DirectoryGatherMessage::TraverseDirectory { parent_dir, .. } = &mut self;
        *parent_dir = Some(parent);
        self
    }
}

/// Messages on the output queue, collected entries, 'Done' when the queue becomes empty and errors passed up
//#[derive(Debug)] FIXME: openat::Metadata is not Debug
pub enum InventoryEntryMessage {
    /// Passes the path and lightweight data from an openat::Entry, no stat() calls are needed.
    Entry(
        Arc<ObjectPath>,
        Option<openat::SimpleType>,
        openat::metadata_types::ino_t,
    ),
    /// Passes the path and openat::Metadata. The user has to crete the metadata which may involve costly stat() calls.
    Metadata(Arc<ObjectPath>, openat::Metadata),
    /// The Gaterers only pass errors up but try to continue.
    Err(DynError),
    /// Message when the input queues got empty and no gathering thread still processes any data.
    Done,
    //    Shutdown
}

impl Debug for InventoryEntryMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        use InventoryEntryMessage::*;
        match self {
            Entry(path, _, _) => write!(f, "Entry {:?}", path.to_pathbuf()),
            Metadata(path, _) => write!(f, "Metadata {:?}", path.to_pathbuf()),
            Err(err) => write!(f, "Error {:?}", err),
            Done => write!(f, "Done"),
        }
    }
}

impl InventoryEntryMessage {
    /// Returns the path of an message if present
    pub fn path(&self) -> Option<&ObjectPath> {
        use InventoryEntryMessage::*;
        match self {
            Entry(path, _, _) => Some(path),
            Metadata(path, _) => Some(path),
            _ => None,
        }
    }

    /// Returns true when this message is an error message.
    pub fn is_error(&self) -> bool {
        matches!(self, InventoryEntryMessage::Err(_))
    }
}
