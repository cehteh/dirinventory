use std::path::PathBuf;
use std::sync::Arc;

use crate::openat;
use crate::ObjectPath;
use crate::Error;

/// Messages on the input queue, directories to be processed.
#[derive(Debug)]
pub enum DirectoryGatherMessage {
    /// Path and parent handle of a directory to be traversed. The handle to the directory
    /// itself will be opened by the thread processing it.
    TraverseDirectory {
        path:       Arc<ObjectPath>,
        parent_dir: Option<Arc<openat::Dir>>,
    },
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
    Entry(openat::Entry, Arc<ObjectPath>),
    Metadata(openat::Metadata, Arc<ObjectPath>),
    Err(Error),
    Done,
}

impl InventoryEntryMessage {
    pub fn path(&self) -> Option<PathBuf> {
        use InventoryEntryMessage::*;
        match self {
            Entry(_, path) => Some(path.to_pathbuf()),
            Metadata(_, path) => Some(path.to_pathbuf()),
            _ => None,
        }
    }
}
