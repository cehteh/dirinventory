//! The Gatherer manages threads which walking directories. Each sub-directory found
//! is added to the list of directories to process. Files are pre-filtered by size and when
//! pass send to an output queue.
use std::io;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::thread;
use std::cmp::Ordering;
use std::sync::atomic::{self, AtomicUsize};

use crossbeam_channel::{bounded, Receiver, Sender};
#[allow(unused_imports)]
pub use log::{debug, error, info, trace, warn};
use openat::{metadata_types, Dir, SimpleType};

use crate::*;

/// Create a space efficient store for file metadata of files larger than a certain
/// min_blocksize.  This is used to find whcih files to delete first for most space efficient
/// deletion.  There should be only one 'Gatherer' around as it is used to merge hardlinks
/// and needs to have a global picture of all indexed files.
pub struct Gatherer {
    names: InternedNames,

    // message queues
    /// The input PriorityQueue fed with directories to be processed
    dirs_queue:           PriorityQueue<DirectoryGatherMessage, u64>,
    ///
    inventory_send_queue: Sender<InventoryEntryMessage>,

    // config section
    min_blocks: metadata_types::blkcnt_t,

    // stats
    inventory_send_queue_pending: AtomicUsize, /* PLANNED: implement a atomicstats crate for counters/min/max/avg etc */
}

impl Gatherer {
    /// Create an Gatherer. The 'min_blocks' sets a pre-filter for keeping only files
    /// which have more than this much blocks (512 bytes) allocated. Since this inventory is
    /// used for deleting the biggiest files (with all hardlinks to be deleted) first, setting
    /// this blockcount to some reasonably higher number can save a lot memory. Files not
    /// indexed in the directory will get deleted anyway on a later pass.  Returns a Result
    /// tuple with an Arc<Gatherer> and the output queue of gathered objects.
    pub fn new(
        min_blocks: metadata_types::blkcnt_t,
        num_threads: usize,
        inventory_backlog: usize,
    ) -> io::Result<(Arc<Gatherer>, Receiver<InventoryEntryMessage>)> {
        let (inventory_send_queue, receiver) = bounded(inventory_backlog);

        let inventory = Arc::new(Gatherer {
            names: InternedNames::new(),
            dirs_queue: PriorityQueue::new(),
            inventory_send_queue,
            min_blocks,
            inventory_send_queue_pending: AtomicUsize::new(0),
        });

        (0..num_threads).try_for_each(|n| -> io::Result<()> {
            inventory.clone().spawn_dir_thread(n)?;
            Ok(())
        })?;

        Ok((inventory, receiver))
    }

    #[inline(always)]
    fn send_dir(&self, message: DirectoryGatherMessage, prio: u64) {
        self.dirs_queue.send(message, prio);
    }

    #[inline(always)]
    fn send_entry(&self, message: InventoryEntryMessage) {
        self.inventory_send_queue.send(message);
        self.inventory_send_queue_pending
            .fetch_add(1, atomic::Ordering::SeqCst);
    }

    #[inline(always)]
    fn process_entry(
        &self,
        entry: io::Result<openat::Entry>,
        dir: Arc<Dir>,
        path: Arc<ObjectPath>,
    ) -> Result<(), Error> {
        // FIXME: when iterating at a certain depth (before number of file handles running
        // out) then dont keep sub dir handles open in an Arc, needs a different strategy
        // then. (break parent Dir, start with a fresh Dir handle)
        let entry = entry.context("Invalid directory entry")?;
        match entry.simple_type() {
            Some(SimpleType::Dir) => {
                trace!("dir: {:?}", path.to_pathbuf().join(entry.file_name()));
                let subdir = ObjectPath::subobject(path, self.names.interning(entry.file_name()));

                // The Order of directory traversal is defined by the 64bit priority in the
                // PriorityQueue. This 64bit are composed of the inode number added directory
                // depth inversed from u64::MAX down shifted by 48 bits (resulting in the
                // upper 16bits for the priority). This results in that directories are
                // traversed depth first in inode increasing order.
                let dir_prio = ((u16::MAX - subdir.depth()) as u64) << 48;
                let message = DirectoryGatherMessage::new_dir(subdir);

                self.send_dir(message.with_parent(dir), dir_prio + entry.inode());
            }
            // TODO: split here on simple-type
            _ => {
                // handle anything else
                let metadata = dir.metadata(entry.file_name()).with_context(|| {
                    format!(
                        "Failed get metadata on: {:?}",
                        path.to_pathbuf().join(entry.file_name())
                    )
                })?;

                trace!("file: {:?}", path.to_pathbuf().join(entry.file_name()));

                // default to zero blocks when not available means it will be filtered out
                let blocks = metadata.blocks().unwrap_or(0);

                // if blocks > self.min_blocks {
                //     self.send_entry(InventoryEntryMessage::Entry(
                //         metadata.dev().unwrap_or(0),
                //         InventoryKey::new(blocks, entry.inode()),
                //         ObjectPath::subobject(path, self.names.interning(entry.file_name())),
                //     ));
                // }
            }
        }
        Ok(())
    }

    /// sends error to output channel and returns it
    fn send_error<T>(&self, err: Error) {
        error!("{:?}", err);
        self.send_entry(InventoryEntryMessage::Err(err));
    }

    fn spawn_dir_thread(self: Arc<Self>, n: usize) -> io::Result<thread::JoinHandle<()>> {
        thread::Builder::new()
            .name(format!("rmrfd/gather/{}", n))
            .spawn(move || {
                loop {
                    use DirectoryGatherMessage::*;

                    match self.dirs_queue.recv().entry() {
                        QueueEntry::Entry(TraverseDirectory { path, parent_dir }, _prio) => {
                            match &parent_dir {
                                // TODO: into function, check if fadvise improves performance
                                // TODO: when out of file handles then reinsert into queue and sleep this thread for 50ms (or find a better way to notify when to continue)
                                Some(dir) => dir.sub_dir(path.name()),
                                None => openat::Dir::open(&path.to_pathbuf()),
                            }
                            .map(|dir| {
                                trace!(
                                    "opened fd {:?}: for {:?}: depth {}",
                                    dir,
                                    path.to_pathbuf(),
                                    path.depth()
                                );
                                let dir = Arc::new(dir);
                                dir.list_self()
                                    .map(|dir_iter| {
                                        dir_iter.for_each(|entry| {
                                            self.process_entry(entry, dir.clone(), path.clone())
                                                .context("Could not process entry")
                                                .map_err(|e| self.send_error::<()>(e))
                                                .ok();
                                        })
                                    })
                                    .map_err(|err| {
                                        error!("{:?}: {:?}", *dir, err);
                                        err
                                    })
                                    .with_context(|| {
                                        format!(
                                            "{:?}: Could not iterate {:?}",
                                            *dir,
                                            path.to_pathbuf()
                                        )
                                    })
                                    .map_err(|e| self.send_error::<()>(e))
                            })
                            .with_context(|| format!("Could not open: {:?}", path.to_pathbuf()))
                            .map_err(|e| self.send_error::<()>(e))
                            .ok();
                        }
                        QueueEntry::Drained => {
                            trace!("drained!!!");
                            self.send_entry(InventoryEntryMessage::Done);
                        }
                        _ => unreachable!(),
                    }
                }
            })
    }

    /// Adds a directory to the processing queue of the inventory.
    pub fn load_dir_recursive(&self, path: Arc<ObjectPath>) {
        self.send_dir(
            DirectoryGatherMessage::new_dir(path),
            u64::MAX, // initial message priority instead depth/inode calculation, added directories are processed at the lowest priority
        );
    }
}

#[derive(Debug)]
pub struct InventoryKey {
    blocks: metadata_types::blkcnt_t,
    ino:    metadata_types::ino_t,
}

impl InventoryKey {
    fn new(blocks: metadata_types::blkcnt_t, ino: metadata_types::ino_t) -> InventoryKey {
        InventoryKey { blocks, ino }
    }
}

impl Ord for InventoryKey {
    fn cmp(&self, other: &Self) -> Ordering {
        let r = self.blocks.cmp(&other.blocks);
        if r == Ordering::Equal {
            self.ino.cmp(&other.ino)
        } else {
            r
        }
    }
}

impl PartialOrd for InventoryKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for InventoryKey {
    fn eq(&self, other: &Self) -> bool {
        self.blocks == other.blocks && self.ino == other.ino
    }
}

impl Eq for InventoryKey {}

#[cfg(test)]
mod test {
    #[allow(unused_imports)]
    pub use log::{debug, error, info, trace, warn};

    use super::*;

    // tests
    #[test]
    fn smoke() {
        crate::test::init_env_logging();
        let _ = Gatherer::new(64, 1, 128);
    }

    #[test]
    #[ignore]
    fn load_dir() {
        crate::test::init_env_logging();

        let (inventory, receiver) = Gatherer::new(64, 16, 65536).unwrap();
        inventory.load_dir_recursive(ObjectPath::new("."));

        let mut out = std::path::PathBuf::new();

        let handles: Vec<_> = (0..4)
            .map(|n| {
                let receiver = receiver.clone();
                let inventory = inventory.clone();
                thread::Builder::new()
                    .name(format!("rmrfd/out/{}", n))
                    .spawn(move || {
                        loop {
                            receiver
                                .iter()
                                .take_while(|msg| !matches!(msg, InventoryEntryMessage::Done))
                                .for_each(|msg| {
                                    let used = inventory
                                        .inventory_send_queue_pending
                                        .fetch_sub(1, atomic::Ordering::SeqCst);
                                    debug!("used {}", used);
                                    // trace!("msg {:?}", msg);
                                });
                        }
                    })
                    .unwrap()
            })
            .collect();

        handles.into_iter().for_each(|h| {
            h.join();
        });

        // let joinme = thread::Builder::new()
        //     .name(format!(
        //         "output_{}",
        //         inventory
        //             .thread_count
        //             .fetch_add(1, atomic::Ordering::Relaxed)
        //     ))
        //     .spawn(move || {
        //         loop {
        //             receiver
        //                 .iter()
        //                 .take_while(|msg| !matches!(msg, InventoryEntryMessage::Done))
        //                 .for_each(|msg| {
        //                     let used = inventory
        //                         .inventory_send_queue_pending
        //                         .fetch_sub(1, atomic::Ordering::SeqCst);
        //                     debug!("used {}", used);
        //                     trace!("msg {:?}", msg);
        //                 });
        //         }
        //     })
        //     .unwrap();
        //
        // joinme.join();

        // receiver
        //     .iter()
        //     .take_while(|msg| !matches!(msg, InventoryEntryMessage::Done))
        //     .for_each(|msg| {
        //         let used = inventory
        //             .inventory_send_queue_pending
        //             .fetch_sub(1, atomic::Ordering::SeqCst);
        //         debug!("used {}", used);
        //         trace!("msg {:?}", msg);
        //     });
    }
}
