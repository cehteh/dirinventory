//! The Gatherer manages threads which walking directories. For each element found a custom processor function is called which may
//! add directories back to the list of directories to process and found entries to an output queue.
use std::io;
use std::sync::Arc;
use std::thread;
use std::ops::DerefMut;

use mpmcpq::*;
use crossbeam_channel::{bounded, Receiver, Sender};
#[allow(unused_imports)]
pub use log::{debug, error, info, trace, warn};
use parking_lot::Mutex;

use crate::*;

/// The type of the user supplied closure/function to process entries.  Takes a GathererHandle
/// which defines the API for pushing things back on the Gatherers queues, the raw
/// openat::Entry to be processed, an object to the path of the parent directory and the Dir
/// handle of the parent dir.
pub type ProcessFn = dyn Fn(GathererHandle, ProcessEntry, Option<Arc<Dir>>) + Send + Sync;

/// The ProcessFn is called with this as parameter. It shall match on this and implement the
/// desired actions.
pub enum ProcessEntry {
    /// Either an entry in the filesystem or an error.
    Result(io::Result<openat::Entry>, Arc<ObjectPath>),
    /// The ProcessFn is called with this after all entries of an directory are processed (but
    /// not its subdirectories). This is used to notify that no more entries of the saied
    /// directory are to be expected.
    EndOfDirectory(Arc<ObjectPath>),
}

type GathererStash<'a> = Stash<'a, DirectoryGatherMessage, u64>;

/// Create a space efficient store for file metadata of files larger than a certain
/// min_blocksize.  This is used to find whcih files to delete first for most space efficient
/// deletion.  There should be only one 'Gatherer' around as it is used to merge hardlinks
/// and needs to have a global picture of all indexed files.
pub struct Gatherer {
    /// All file/dir names are interned here
    names: InternedNames<32>,

    /// The processing function
    processor: Box<ProcessFn>,

    // message queues
    /// The input PriorityQueue fed with directories to be processed
    dirs_queue:      PriorityQueue<DirectoryGatherMessage, u64>,
    /// The output channels where the results are send to.
    output_channels: Vec<(
        Sender<InventoryEntryMessage>,
        Arc<Receiver<InventoryEntryMessage>>,
    )>,

    /// Sending an initial directory requires an stash.
    // PLANNED: Also used when one wants to push multiple directories.
    kickoff_stash: Mutex<GathererStash<'static>>,

    /// The maximum number of file descriptors this Gatherer may use.
    fd_limit: usize,

    /// Number of DirectoryGathermessages batched together
    message_batch: usize,
}

impl Gatherer {
    /// Creates a gatherer builder used to configure the gatherer. Uses conservative defaults,
    /// 16 threads and 64k backlog.
    #[must_use = "configure the Gatherer and finally call .start()"]
    pub fn build() -> GathererBuilder {
        GathererBuilder::new()
    }

    /// Returns the an Arc of the receiver side of output channel 'n'.
    pub fn channel(&self, n: usize) -> Arc<Receiver<InventoryEntryMessage>> {
        self.output_channels[n].1.clone()
    }

    /// Returns the number of output channels.
    pub fn num_channels(&self) -> usize {
        self.output_channels.len()
    }

    /// Returns a Vec with all receiving sides of the output channels.
    pub fn channels_as_vec(&self) -> Vec<Arc<Receiver<InventoryEntryMessage>>> {
        self.output_channels
            .iter()
            .map(|(_, r)| r.clone())
            .collect()
    }

    /// Adds a directory to the processing queue of the inventory. This is the main function
    /// to initiate a directory traversal.
    pub fn load_dir_recursive(&self, path: Arc<ObjectPath>) {
        let mut stash = self.kickoff_stash.lock();
        self.send_dir(
            DirectoryGatherMessage::new_dir(path),
            u64::MAX, /* initial message priority instead depth/inode calculation, added
                       * directories are processed at the lowest priority */
            stash.deref_mut(),
        );
        self.dirs_queue.sync(stash.deref_mut());
    }

    // TODO: fn shutdown, there is currently no way to free a Gatherer as the threads keep it alive

    /// put a DirectoryGatherMessage on the input queue (traverse sub directories).
    #[inline(always)]
    fn send_dir(&self, message: DirectoryGatherMessage, prio: u64, stash: &GathererStash) {
        self.dirs_queue
            .send_batched(message, prio, self.message_batch, stash);
    }

    /// Put a message on an output channel.
    #[inline(always)]
    fn send_entry(&self, channel: usize, message: InventoryEntryMessage) {
        // Ignore result, the user may have dropped the receiver, but there is nothing we
        // should do about it.
        let _ = self.output_channels[channel].0.send(message);
    }

    fn resend_dir(&self, message: DirectoryGatherMessage, prio: u64, stash: &GathererStash) {
        self.send_dir(message, prio, stash);
        thread::sleep(std::time::Duration::from_millis(5));
    }

    /// Spawns a single gatherer thread
    fn spawn_gather_thread(self: Arc<Self>, n: usize) -> io::Result<thread::JoinHandle<()>> {
        thread::Builder::new()
            .name(format!("dir/gather/{}", n))
            .spawn(move || {
                let stash: GathererStash = Stash::new(&self.dirs_queue);
                loop {
                    use DirectoryGatherMessage::*;

                    // TODO: messages for dir enter/leave on the ouput queue
                    match self.dirs_queue.recv_guard().message() {
                        mpmcpq::Message::Msg(TraverseDirectory { path, parent_dir }, prio) => {
                            if used_handles() >= self.fd_limit {
                                warn!("filehandle limit reached");
                                self.resend_dir(
                                    TraverseDirectory {
                                        path:       path.clone(),
                                        parent_dir: parent_dir.clone(),
                                    },
                                    *prio,
                                    &stash,
                                );
                            } else {
                                match parent_dir {
                                    Some(dir) => dir.sub_dir(path.name()),
                                    None => Dir::open(&path.to_pathbuf()),
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
                                                (self.processor)(
                                                    GathererHandle {
                                                        gatherer: &self,
                                                        stash:    &stash,
                                                    },
                                                    ProcessEntry::Result(entry, path.clone()),
                                                    Some(dir.clone()),
                                                );
                                            });

                                            // TODO: use processfn
                                            (self.processor)(
                                                GathererHandle {
                                                    gatherer: &self,
                                                    stash:    &stash,
                                                },
                                                ProcessEntry::EndOfDirectory(path.clone()),
                                                Some(dir.clone()),
                                            );

                                            let _ = self.output_channels[0].0.send(
                                                InventoryEntryMessage::EndOfDirectory {
                                                    path: path.clone(),
                                                },
                                            );

                                            self.dirs_queue.sync(&stash);
                                            crate::dirhandle::dec_handles();
                                        })
                                        .map_err(|err| {
                                            if err.raw_os_error() == Some(libc::EMFILE) {
                                                self.resend_dir(
                                                    TraverseDirectory {
                                                        path:       path.clone(),
                                                        parent_dir: parent_dir.clone(),
                                                    },
                                                    *prio,
                                                    &stash,
                                                );
                                            } else {
                                                (self.processor)(
                                                    GathererHandle {
                                                        gatherer: &self,
                                                        stash:    &stash,
                                                    },
                                                    ProcessEntry::Result(Err(err), path.clone()),
                                                    Some(dir),
                                                );
                                            }
                                        })
                                })
                                .map_err(|err| {
                                    if err.raw_os_error() == Some(libc::EMFILE) {
                                        warn!("filehandles exhausted");
                                        self.resend_dir(
                                            TraverseDirectory {
                                                path:       path.clone(),
                                                parent_dir: parent_dir.clone(),
                                            },
                                            *prio,
                                            &stash,
                                        );
                                    } else {
                                        (self.processor)(
                                            GathererHandle {
                                                gatherer: &self,
                                                stash:    &stash,
                                            },
                                            ProcessEntry::Result(Err(err), path.clone()),
                                            parent_dir.clone(),
                                        );
                                    }
                                })
                                .ok();
                            }
                        }
                        mpmcpq::Message::Drained => {
                            trace!("drained!!!");
                            (0..self.output_channels.len())
                                .for_each(|n| self.send_entry(n, InventoryEntryMessage::Done));
                        }
                        _ => unimplemented!(),
                    }
                }
            })
    }
}

/// Configures a Gatherer
pub struct GathererBuilder {
    num_gather_threads:  usize,
    num_output_channels: usize,
    inventory_backlog:   usize,
    fd_limit:            usize,
    message_batch:       usize,
}

impl Default for GathererBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GathererBuilder {
    fn new() -> Self {
        GathererBuilder {
            num_gather_threads:  16,
            num_output_channels: 1,
            inventory_backlog:   0,
            fd_limit:            512,
            message_batch:       512,
        }
    }

    /// Starts the Gatherer. Takes the user defined processing function as argument. This
    /// function is used to process every directory entry seen. It should be small and fast
    /// selecting which sub directories to be traversed and which entries to pass to the
    /// output channels. Any more work should be done on the output then.
    pub fn start(&self, processor: Box<ProcessFn>) -> io::Result<Arc<Gatherer>> {
        let output_channels = (0..self.num_output_channels)
            .map(|_| {
                let (sender, receiver) = bounded(
                    // when inventory backlog is not set, set it automatically to 4k per thread
                    if self.inventory_backlog == 0 {
                        4096 * self.num_gather_threads
                    } else {
                        self.inventory_backlog
                    },
                );
                (sender, Arc::new(receiver))
            })
            .collect();

        let gatherer = Arc::new(Gatherer {
            names: InternedNames::new(),
            dirs_queue: PriorityQueue::new(),
            output_channels,
            processor,
            kickoff_stash: Mutex::new(GathererStash::new_without_priority_queue()),
            fd_limit: self.fd_limit,
            message_batch: self.message_batch,
        });

        (0..self.num_gather_threads).try_for_each(|n| -> io::Result<()> {
            gatherer.clone().spawn_gather_thread(n)?;
            Ok(())
        })?;

        debug!("created gatherer");
        Ok(gatherer)
    }

    /// Sets the number of threads which traverse the directories. These are IO-bound
    /// operations and the more threads are used the better are the opportunities for the
    /// kernel to optimize IO-Requests. Tests have shown that on fast SSD's and cached data
    /// thread numbers in the hundrededs still show some benefits (at high resource
    /// costs). For general operation and on slower HDD's / non cached data 8-64 threads
    /// should be good enough. Default is 16 threads.
    #[must_use = "GathererBuilder must be used, call .start()"]
    pub fn with_gather_threads(mut self, num_threads: usize) -> Self {
        assert!(num_threads > 0, "Must at least use one thread");
        self.num_gather_threads = num_threads;
        self
    }

    /// Sets the amount of messages the output channels can hold. For cached and readahead data,
    /// the kernel can send bursts entries to the gatherer threads at very high speeds, since
    /// we don't want to stall the gathering, the is adds some output buffering. Usually
    /// values from 64k to 512k should be fine here. When zero (the default) 4k per thread are
    /// used.
    #[must_use = "GathererBuilder must be used, call .start()"]
    pub fn with_inventory_backlog(mut self, backlog_size: usize) -> Self {
        self.inventory_backlog = backlog_size;
        self
    }

    /// Sets the maximum number of directory handles the Gatherer may use. The Gatherer has a
    /// build-in strategy handle fd exhaustion when this happens earlier, but keep in mind
    /// that then there are no fd's for the other parts of the application
    /// available. Constraining the number of file handles too much will make its slow and
    /// eventually deadlock. Limit them to no less than num_threads+100 handles! The limits
    /// are not enforced since the actual amount needed depends a lot factors. Defaults to 512
    /// fd's which should be plenty for most cases.
    #[must_use = "GathererBuilder must be used, call .start()"]
    pub fn with_fd_limit(mut self, fd_limit: usize) -> Self {
        self.fd_limit = fd_limit;
        self
    }

    /// Sets size of message batched together. This reduces contention on the priority
    /// lock. While it won't improve performance it can reduce the CPU load (often
    /// insignificantly). Defaults to 512, shouldn't need adjustments except for benchmarking.
    #[must_use = "GathererBuilder must be used, call .start()"]
    pub fn with_message_batch(mut self, message_batch: usize) -> Self {
        self.message_batch = message_batch;
        self
    }
}

/// Defines the API the user defined ProcessFn may use to send data back on the
/// input queue and output channels.
pub struct GathererHandle<'a> {
    gatherer: &'a Gatherer,
    stash:    &'a GathererStash<'a>,
}

impl GathererHandle<'_> {
    /// Add a sub directory to the input priority queue to be traversed as well.
    pub fn traverse_dir(
        &self,
        entry: &openat::Entry,
        parent_path: Arc<ObjectPath>,
        parent_dir: Option<Arc<Dir>>,
    ) {
        let subdir = ObjectPath::subobject(
            parent_path,
            self.gatherer.names.interning(entry.file_name()),
        );

        // The Order of directory traversal is defined by the 64bit priority in the
        // PriorityQueue. This 64bit are composed of the inode number added directory
        // depth inversed from u64::MAX down shifted by 48 bits (resulting in the
        // upper 16bits for the priority). This results in that directories are
        // traversed depth first in inode increasing order.
        // PLANNED: When deeper than 64k consider it as loop? do a explicit loop check?
        let dir_prio = ((u16::MAX - subdir.depth()) as u64) << 48;
        let message = DirectoryGatherMessage::new_dir(subdir);

        self.gatherer.send_dir(
            message.with_parent_dir(parent_dir),
            dir_prio + entry.inode(),
            self.stash,
        );
    }

    /// Sends openat::Entry components to the output channel.
    pub fn output_entry(
        &self,
        channel: usize,
        entry: &openat::Entry,
        parent_path: Arc<ObjectPath>,
    ) {
        let path = ObjectPath::subobject(
            parent_path,
            self.gatherer.names.interning(entry.file_name()),
        );
        self.gatherer
            .send_entry(channel, InventoryEntryMessage::Entry {
                path,
                file_type: entry.simple_type(),
                inode: entry.inode(),
            });
    }

    /// Sends openat::Metadata to the output channel
    pub fn output_metadata(
        &self,
        channel: usize,
        entry: &openat::Entry,
        parent_path: Arc<ObjectPath>,
        metadata: openat::Metadata,
    ) {
        let entryname = ObjectPath::subobject(
            parent_path,
            self.gatherer.names.interning(entry.file_name()),
        );
        self.gatherer
            .send_entry(channel, InventoryEntryMessage::Metadata {
                path: entryname,
                metadata,
            });
    }

    /// Sends an error to the output channel
    pub fn output_error(&self, channel: usize, error: DynError, path: Arc<ObjectPath>) {
        warn!("{:?} at {:?}", error, path);
        self.gatherer
            .send_entry(channel, InventoryEntryMessage::Err { path, error });
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
        let _ = Gatherer::build().with_gather_threads(1).start(Box::new(
            |_gatherer: GathererHandle, _entry: ProcessEntry, _parent_dir: Option<Arc<Dir>>| {},
        ));
    }

    #[test]
    #[ignore]
    fn load_dir() {
        crate::test::init_env_logging();

        let gatherer = Gatherer::build()
            .with_gather_threads(128)
            .with_fd_limit(768)
            .start(Box::new(
                |gatherer: GathererHandle, entry: ProcessEntry, parent_dir: Option<Arc<Dir>>| {
                    match entry {
                        ProcessEntry::Result(Ok(entry), parent_path) => match entry.simple_type() {
                            Some(openat::SimpleType::Dir) => {
                                gatherer.traverse_dir(
                                    &entry,
                                    parent_path.clone(),
                                    parent_dir.clone(),
                                );
                                gatherer.output_entry(0, &entry, parent_path.clone());
                            }
                            _ => {
                                gatherer.output_entry(0, &entry, parent_path);
                            }
                        },
                        ProcessEntry::Result(Err(err), parent_path) => {
                            gatherer.output_error(0, Box::new(err), parent_path);
                        }
                        _ => {}
                    }
                },
            ))
            .unwrap();

        gatherer.load_dir_recursive(ObjectPath::new("."));

        let mut stdout = std::io::stdout();

        gatherer
            .channel(0)
            .iter()
            .take_while(|msg| !matches!(msg, InventoryEntryMessage::Done))
            .for_each(|msg| {
                if let Some(path) = msg.path() {
                    let _ = stdout.write_all(path.to_pathbuf().as_os_str().as_bytes());
                    let _ = stdout.write_all(b"\n");
                } else if msg.is_error() {
                    error!("{:?}", msg)
                }
            });
    }

    #[test]
    fn entry_messages() {
        crate::test::init_env_logging();

        let gatherer = Gatherer::build()
            .start(Box::new(
                |gatherer: GathererHandle, entry: ProcessEntry, parent_dir: Option<Arc<Dir>>| {
                    match entry {
                        ProcessEntry::Result(Ok(entry), parent_path) => match entry.simple_type() {
                            Some(openat::SimpleType::Dir) => {
                                gatherer.traverse_dir(&entry, parent_path, parent_dir);
                            }
                            _ => {
                                gatherer.output_entry(0, &entry, parent_path);
                            }
                        },
                        ProcessEntry::Result(Err(err), parent_path) => {
                            gatherer.output_error(0, Box::new(err), parent_path);
                        }
                        _ => {}
                    }
                },
            ))
            .unwrap();

        gatherer.load_dir_recursive(ObjectPath::new("src"));

        let mut stdout = std::io::stdout();

        gatherer
            .channel(0)
            .iter()
            .take_while(|msg| !matches!(msg, InventoryEntryMessage::Done))
            .for_each(|msg| {
                if let Some(path) = msg.path() {
                    let _ = stdout.write_all(path.to_pathbuf().as_os_str().as_bytes());
                    let _ = stdout.write_all(b"\n");
                }
            });
    }

    #[test]
    fn metadata_messages() {
        crate::test::init_env_logging();

        let gatherer = Gatherer::build()
            .start(Box::new(
                |gatherer: GathererHandle, entry: ProcessEntry, parent_dir: Option<Arc<Dir>>| {
                    match entry {
                        ProcessEntry::Result(Ok(entry), parent_path) => match entry.simple_type() {
                            Some(openat::SimpleType::Dir) => {
                                gatherer.traverse_dir(&entry, parent_path.clone(), parent_dir);
                            }
                            _ => match parent_dir.clone().unwrap().metadata(entry.file_name()) {
                                Ok(metadata) => {
                                    gatherer.output_metadata(0, &entry, parent_path, metadata);
                                }
                                Err(err) => {
                                    gatherer.output_error(0, Box::new(err), parent_path);
                                }
                            },
                        },
                        ProcessEntry::Result(Err(err), parent_path) => {
                            gatherer.output_error(0, Box::new(err), parent_path);
                        }
                        _ => {}
                    }
                },
            ))
            .unwrap();
        gatherer.load_dir_recursive(ObjectPath::new("src"));

        let mut stdout = std::io::stdout();

        gatherer
            .channel(0)
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
