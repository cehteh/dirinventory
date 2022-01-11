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
        parent_dir: Option<Arc<Dir>>,
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
    pub fn with_parent_dir(mut self, parent: Option<Arc<Dir>>) -> Self {
        debug_assert!(matches!(
            self,
            DirectoryGatherMessage::TraverseDirectory { .. }
        ));
        let DirectoryGatherMessage::TraverseDirectory { parent_dir, .. } = &mut self;
        *parent_dir = parent;
        self
    }
}

/// Messages on the output queue, collected entries, 'Done' when the queue becomes empty and
/// errors passed up
//#[derive(Debug)] FIXME: openat::Metadata is not Debug
pub enum InventoryEntryMessage {
    /// Passes the path and lightweight data from an openat::Entry, no stat() calls are needed.
    Entry {
        /// Filename of this entry.
        path:      Arc<ObjectPath>,
        /// Type of file.
        file_type: Option<openat::SimpleType>,
        /// Inode number.
        inode:     openat::metadata_types::ino_t,
    },
    /// Passes the path and openat::Metadata. The user has to crete the metadata which may
    /// involve costly stat() calls.
    Metadata {
        /// Filename of this entry.
        path:     Arc<ObjectPath>,
        /// Metadata for this entry.
        metadata: openat::Metadata,
    },
    /// Send for each Directory when its processing is completed to let the receiver on the
    /// output know that no more data for this directory will be send.
    EndOfDirectory {
        /// Filename of this entry.
        path: Arc<ObjectPath>,
    },
    /// The Gaterers only pass errors up but try to continue.
    Err {
        /// Filename of this entry.
        path:  Arc<ObjectPath>,
        /// The error.
        error: DynError,
    },
    /// Message when the input queues got empty and no gathering thread still processes any
    /// data.
    Done,
    //    Shutdown
}

impl Debug for InventoryEntryMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        use InventoryEntryMessage::*;
        match self {
            Entry { path, .. } => write!(f, "Entry {:?}", path.to_pathbuf()),
            Metadata { path, .. } => write!(f, "Metadata {:?}", path.to_pathbuf()),
            EndOfDirectory { path, .. } => write!(f, "EndOfDirectory {:?}", path.to_pathbuf()),
            Err { path, error, .. } => write!(f, "Error {:?} at {:?}", error, path.to_pathbuf()),
            Done => write!(f, "Done"),
        }
    }
}

impl InventoryEntryMessage {
    /// Returns the path of an message if present
    pub fn path(&self) -> Option<&ObjectPath> {
        use InventoryEntryMessage::*;
        match self {
            Entry { path, .. } => Some(path),
            Metadata { path, .. } => Some(path),
            _ => None,
        }
    }

    /// Returns true when this message is an error message.
    pub fn is_error(&self) -> bool {
        matches!(self, InventoryEntryMessage::Err { .. })
    }
}
