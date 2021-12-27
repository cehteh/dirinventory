//! The Gatherer manages threads which walking directories. For each element found a custom processor function is called which may
//! add directories back to the list of directories to process and found entries to an output queue.
use std::io;
use std::sync::Arc;
use std::thread;

use crossbeam_channel::{bounded, Receiver, Sender};
#[allow(unused_imports)]
pub use log::{debug, error, info, trace, warn};

use crate::*;

// The type of the user supplied closure/function to process entries.  Takes a GathererHandle
// which defines the API for pushing things back on the Gatherers queues, the raw
// openat::Entry to be processed, an object to the path of the parent directory and the openat::Dir
// handle of the parent dir.
type ProcessFn = dyn Fn(GathererHandle, openat::Entry, Arc<ObjectPath>, Arc<openat::Dir>) -> Result<(), Error>
    + Send
    + Sync;

/// Create a space efficient store for file metadata of files larger than a certain
/// min_blocksize.  This is used to find whcih files to delete first for most space efficient
/// deletion.  There should be only one 'Gatherer' around as it is used to merge hardlinks
/// and needs to have a global picture of all indexed files.
pub struct Gatherer {
    /// All file/dir names are interned here
    names: InternedNames,

    /// The processing function
    processor: &'static ProcessFn,

    // message queues
    /// The input PriorityQueue fed with directories to be processed
    dirs_queue:           PriorityQueue<DirectoryGatherMessage, u64>,
    /// The output queue where the results are send to.
    inventory_send_queue: Sender<InventoryEntryMessage>,
}

impl Gatherer {
    /// Create an Gatherer.  'num_gather_threads' is the number of threads which traverse the
    /// directories. These are IO-bound operations and the more threads are used the better
    /// are the opportunities for the kernel to optimize IO-Requests. Tests have shown that on
    /// fast SSD's and cached data thread numbers in the hundrededs still show some benefits
    /// (at high resource costs). For general operation and on slower HDD's / non cached data
    /// 8-64 threads should be good enough. The 'inventory_backlog' is the about of entries
    /// the bounded output queue can hold. For cached and readahead data, the kernel can send
    /// bursts entries to the gatherer threads at very high speeds, since we don't want to
    /// stall the gathering, the is adds some output buffering. Usually values from 64k to
    /// 512k should be fine here.
    ///
    /// Returns a Result tuple with an Arc<Gatherer> and the receiving end of the output queue.
    pub fn new(
        num_gather_threads: usize,
        inventory_backlog: usize,
        processor: &'static ProcessFn,
    ) -> io::Result<(Arc<Gatherer>, Receiver<InventoryEntryMessage>)> {
        let (inventory_send_queue, receiver) = bounded(inventory_backlog);

        let gatherer = Arc::new(Gatherer {
            names: InternedNames::new(),
            dirs_queue: PriorityQueue::new(),
            inventory_send_queue,
            processor,
        });

        (0..num_gather_threads).try_for_each(|n| -> io::Result<()> {
            gatherer.clone().spawn_gather_thread(n)?;
            Ok(())
        })?;

        debug!("created gatherer");
        Ok((gatherer, receiver))
    }

    /// Adds a directory to the processing queue of the inventory. This is the main function
    /// to initiate a directory traversal.
    pub fn load_dir_recursive(&self, path: Arc<ObjectPath>) {
        self.send_dir(
            DirectoryGatherMessage::new_dir(path),
            u64::MAX, /* initial message priority instead depth/inode calculation, added
                       * directories are processed at the lowest priority */
        );
    }

    // TODO: fn shutdown, there is currently no way to free a Gatherer as the threads keep it alive

    /// put a message on the input queue.
    #[inline(always)]
    fn send_dir(&self, message: DirectoryGatherMessage, prio: u64) {
        self.dirs_queue.send(message, prio);
    }

    /// put a message on the output queue.
    #[inline(always)]
    fn send_entry(&self, message: InventoryEntryMessage) {
        // Ignore result, the user may have dropped the receiver, but there is nothing we
        // should do about it.
        let _ = self.inventory_send_queue.send(message);
    }

    /// sends error to output channel and returns it
    fn send_error<T>(&self, err: Error) {
        warn!("{:?}", err);
        self.send_entry(InventoryEntryMessage::Err(err));
    }

    /// Called for each entry (including errorneous) found. Calls the user-provided process
    /// function.
    fn process_entry(
        &self,
        entry: io::Result<openat::Entry>,
        parent_path: Arc<ObjectPath>,
        parent_dir: Arc<openat::Dir>,
    ) -> Result<(), Error> {
        let entry = entry.context("Invalid directory entry")?;
        (self.processor)(GathererHandle(self), entry, parent_path, parent_dir)?;
        Ok(())
    }

    /// Spawns a single gatherer thread
    fn spawn_gather_thread(self: Arc<Self>, n: usize) -> io::Result<thread::JoinHandle<()>> {
        thread::Builder::new()
            .name(format!("dir/gather/{}", n))
            .spawn(move || {
                loop {
                    use DirectoryGatherMessage::*;

                    // TODO: messages for dir enter/leave on the ouput queue
                    // FIXME: when iterating at a certain depth (before number of file handles running
                    // out) then dont keep sub dir handles open in an Arc, needs a different strategy
                    // then. (break parent Dir, start with a fresh Dir handle)
                    match self.dirs_queue.recv().entry() {
                        QueueEntry::Entry(TraverseDirectory { path, parent_dir }, _prio) => {
                            match &parent_dir {
                                // TODO: into function, check if fadvise improves performance
                                // TODO: when out of file handles then reinsert into queue and
                                // sleep this thread for 50ms (or find a better way to notify
                                // when to continue)
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
                                            self.process_entry(entry, path.clone(), dir.clone())
                                                .context("Could not process entry")
                                                .map_err(|e| self.send_error::<()>(e))
                                                .ok();
                                        })
                                    })
                                    .map_err(|err| {
                                        warn!("{:?}: {:?}", *dir, err);
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
                        _ => unimplemented!(),
                    }
                }
            })
    }
}

// Defines the API the user defined processor function may use to send data back on the
// queues.
pub struct GathererHandle<'a>(&'a Gatherer);

impl GathererHandle<'_> {
    /// Add a sub directory to the input priority queue to be traversed as well.
    pub fn traverse_dir(
        &self,
        entry: Arc<openat::Entry>,
        parent_path: Arc<ObjectPath>,
        parent_dir: Arc<openat::Dir>,
    ) {
        let subdir = ObjectPath::subobject(parent_path, self.0.names.interning(entry.file_name()));

        // The Order of directory traversal is defined by the 64bit priority in the
        // PriorityQueue. This 64bit are composed of the inode number added directory
        // depth inversed from u64::MAX down shifted by 48 bits (resulting in the
        // upper 16bits for the priority). This results in that directories are
        // traversed depth first in inode increasing order.
        let dir_prio = ((u16::MAX - subdir.depth()) as u64) << 48;
        let message = DirectoryGatherMessage::new_dir(subdir);

        self.0
            .send_dir(message.with_parent(parent_dir), dir_prio + entry.inode());
    }

    /// Sends an openat::Entry found to the output queue
    pub fn output_entry(&self, entry: Arc<openat::Entry>, parent_path: Arc<ObjectPath>) {
        let entryname =
            ObjectPath::subobject(parent_path, self.0.names.interning(entry.file_name()));
        self.0
            .send_entry(InventoryEntryMessage::Entry(entry, entryname));
    }
}

#[cfg(test)]
mod test {
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;

    #[allow(unused_imports)]
    pub use log::{debug, error, info, trace, warn};

    use super::*;

    // tests
    #[test]
    fn smoke() {
        crate::test::init_env_logging();
        let _ = Gatherer::new(1, 65536, &|_gatherer: GathererHandle,
                                          _entry: openat::Entry,
                                          _parent_path: Arc<ObjectPath>,
                                          _parent_dir: Arc<openat::Dir>|
         -> Result<(), Error> { Ok(()) });
    }

    #[test]
    #[ignore]
    fn load_dir() {
        crate::test::init_env_logging();

        let (inventory, receiver) = Gatherer::new(16, 65536, &|gatherer: GathererHandle,
                                                               entry: openat::Entry,
                                                               parent_path: Arc<ObjectPath>,
                                                               parent_dir: Arc<openat::Dir>|
         -> Result<(), Error> {
            match entry.simple_type() {
                Some(openat::SimpleType::Dir) => {
                    let entry = Arc::new(entry);
                    gatherer.traverse_dir(entry.clone(), parent_path.clone(), parent_dir.clone());
                    gatherer.output_entry(entry, parent_path);
                }
                _ => {
                    gatherer.output_entry(Arc::new(entry), parent_path);
                }
            }
            Ok(())
        })
        .unwrap();
        inventory.load_dir_recursive(ObjectPath::new("."));

        let mut stdout = std::io::stdout();

        receiver
            .iter()
            .take_while(|msg| !matches!(msg, InventoryEntryMessage::Done))
            .for_each(|msg| {
                if let Some(path) = msg.path() {
                    let _ = stdout.write_all(path.to_pathbuf().as_os_str().as_bytes());
                    let _ = stdout.write_all(b"\n");
                }
            });
    }
}
