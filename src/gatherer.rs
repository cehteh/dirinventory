//! The Gatherer manages threads which walking directories. For each element found a custom processor function is called which may
//! add directories back to the list of directories to process and found entries to an output queue.
use std::io;
use std::sync::{Arc, Weak};
use std::thread;
use std::ops::DerefMut;
use std::path::Path;
use std::ffi::OsStr;

use mpmcpq::*;
use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::Mutex;

#[allow(unused_imports)]
use crate::{debug, error, info, trace, warn};
use crate::*;

/// The type of the user supplied closure/function to process entries.  Takes a GathererHandle
/// which defines the API for pushing things back on the Gatherers queues and a ProcessMessage
/// which wraps the things to be handled.
// PLANNED: Dir will become part of ObjectPath
pub type ProcessFn = dyn Fn(GathererHandle, ProcessMessage) + Send + Sync;

/// The input to the ProcessFn which dispatches actions on the queues via the GathererHandle.
pub enum ProcessMessage {
    /// Either an openat::Entry for iterated directory entries or an error.
    Result(io::Result<openat::Entry>, ObjectPath),
    /// A notification that a ObjectPath processing is finished.
    ObjectDone(ObjectPath),
    /// The ProcessFn is called with this after all entries of an directory are processed (but
    /// not its subdirectories). This is used to notify that no more entries of the saied
    /// directory are to be expected. Note that ObjectDone messages may still appear.
    EndOfDirectory(ObjectPath),
}

pub(crate) type GathererStash<'a> = Stash<'a, DirectoryGatherMessage, u64>;

/// The Gatherer manages the queues and threads traversing directories.
/// There should be only one 'Gatherer' around as it is used to merge hardlinks
/// and needs to have a global picture of all indexed files.
pub struct Gatherer(Arc<GathererInner>);

impl Gatherer {
    /// Creates a gatherer builder used to configure the gatherer. Uses conservative defaults,
    /// 16 threads and 64k backlog.
    #[must_use = "configure the Gatherer and finally call .start()"]
    pub fn build() -> GathererBuilder {
        GathererBuilder::new()
    }

    /// Try to upgrade a Weak reference into a Gatherer.
    pub(crate) fn upgrade(weak: &Weak<GathererInner>) -> Option<Gatherer> {
        weak.upgrade().map(Gatherer)
    }

    /// Get a Weak reference from a Gatherer.
    pub(crate) fn downgrade(&self) -> Weak<GathererInner> {
        Arc::downgrade(&self.0)
    }

    /// Access to the inner data.
    pub(crate) fn inner(&self) -> &GathererInner {
        &*self.0
    }

    /// Returns the an Arc of the receiver side of output channel 'n'.
    pub fn channel(&self, n: usize) -> Arc<Receiver<InventoryEntryMessage>> {
        self.0.output_channels[n].1.clone()
    }

    /// Returns the number of output channels.
    pub fn num_channels(&self) -> usize {
        self.0.output_channels.len() as usize
    }

    /// Returns a Vec with all receiving sides of the output channels.
    pub fn channels_as_vec(&self) -> Vec<Arc<Receiver<InventoryEntryMessage>>> {
        self.0
            .output_channels
            .iter()
            .map(|(_, r)| r.clone())
            .collect()
    }

    /// Adds a name to the interned names, returns that.
    pub fn name_interning(&self, name: &OsStr) -> InternedName {
        self.0.names.interning(name)
    }

    /// Adds a directory to the processing queue of the inventory. This is the main function
    /// to initiate a directory traversal.
    // pub fn load_dir_recursive(&self, path: ObjectPath) {
    pub fn load_dir_recursive<P: AsRef<Path>>(&self, path: P, watch: bool) {
        let path = ObjectPath::new(path, self);
        if let Ok(dir) = path.dir() {
            self.0.dirs_lru.preserve(dir);
        };

        path.watch(watch);

        let mut stash = self.0.kickoff_stash.lock();
        self.0.send_dir(
            DirectoryGatherMessage::new_dir(path),
            u64::MAX, /* initial message priority instead depth/inode calculation, added
                       * directories are processed at the lowest priority */
            Some(stash.deref_mut()),
        );
        self.0.dirs_queue.sync(stash.deref_mut());
    }

    // TODO: fn shutdown, there is currently no way to free a Gatherer as the threads keep it alive

    /// called when a watched ObjectPath's strong_count becomes dropped equal or below 2.
    pub(crate) fn notify_path_dropped(&self, path: ObjectPath) {
        (self.0.processor)(
            GathererHandle::new(
                // infallible since caller checked already
                &path.gatherer().unwrap(),
                None,
            ),
            ProcessMessage::ObjectDone(path),
        );
    }

    /// Spawns a single gatherer thread
    fn spawn_gather_thread(&self, n: usize) -> io::Result<thread::JoinHandle<()>> {
        thread::Builder::new().name(format!("gather/{}", n)).spawn({
            let gatherer = Gatherer(self.0.clone());
            move || {
                debug!("thread started");
                let stash: GathererStash = Stash::new(&gatherer.0.dirs_queue);
                loop {
                    use DirectoryGatherMessage::*;

                    // TODO: messages for dir enter/leave on the ouput queue
                    match gatherer.0.dirs_queue.recv_guard().message() {
                        mpmcpq::Message::Msg(TraverseDirectory { path }, prio) => {
                            gatherer.0.dirs_lru.expire_until(gatherer.0.lru_batch, &|| {
                                used_handles() <= gatherer.0.fd_limit
                            });

                            if used_handles() <= gatherer.0.fd_limit {
                                path.dir()
                                    .map(|dir| {
                                        trace!(
                                            "opened fd {:?}: for {:?}: depth {}",
                                            dir,
                                            path.to_pathbuf(),
                                            path.depth()
                                        );

                                        gatherer.0.dirs_lru.preserve(dir.clone());

                                        // Clone the path to increment the refcount to trigger notification reliably
                                        let path = path.clone();
                                        dir.list_self()
                                            .map(|dir_iter| {
                                                dir_iter.for_each(|entry| {
                                                    (gatherer.0.processor)(
                                                        GathererHandle::new(
                                                            &gatherer,
                                                            Some(&stash),
                                                        ),
                                                        ProcessMessage::Result(entry, path.clone()),
                                                    );
                                                });

                                                (gatherer.0.processor)(
                                                    GathererHandle::new(&gatherer, Some(&stash)),
                                                    ProcessMessage::EndOfDirectory(path.clone()),
                                                );

                                                gatherer.0.dirs_queue.sync(&stash);
                                                // path.adjust_dir();
                                                crate::dirhandle::dec_handles();
                                            })
                                            .map_err(|err| {
                                                if err.raw_os_error() == Some(libc::EMFILE) {
                                                    gatherer.0.resend_dir(
                                                        TraverseDirectory { path: path.clone() },
                                                        *prio,
                                                        &stash,
                                                    );
                                                } else {
                                                    (gatherer.0.processor)(
                                                        GathererHandle::new(
                                                            &gatherer,
                                                            Some(&stash),
                                                        ),
                                                        ProcessMessage::Result(
                                                            Err(err),
                                                            path.clone(),
                                                        ),
                                                    );
                                                }
                                            })
                                    })
                                    .map_err(|err| {
                                        if err.raw_os_error() == Some(libc::EMFILE) {
                                            warn!("filehandles exhausted");
                                            gatherer.0.resend_dir(
                                                TraverseDirectory { path: path.clone() },
                                                *prio,
                                                &stash,
                                            );
                                        } else {
                                            (gatherer.0.processor)(
                                                GathererHandle::new(&gatherer, Some(&stash)),
                                                ProcessMessage::Result(Err(err), path.clone()),
                                            );
                                        }
                                    })
                                    .ok();
                            } else {
                                warn!("filehandle limit reached");
                                gatherer.0.resend_dir(
                                    TraverseDirectory { path: path.clone() },
                                    *prio,
                                    &stash,
                                );
                            }
                        }
                        mpmcpq::Message::Drained => {
                            trace!("drained!!!");
                            (0..gatherer.0.output_channels.len()).for_each(|n| {
                                gatherer.0.send_entry(n, InventoryEntryMessage::Done)
                            });
                        }
                        _ => unimplemented!(),
                    }
                }
            }
        })
    }
}

pub(crate) struct GathererInner {
    /// All file/dir names are interned here
    names: InternedNames<32>,

    /// The processing function
    processor: Box<ProcessFn>,

    // message queues
    /// The input PriorityQueue fed with directories to be processed
    dirs_queue:      PriorityQueue<DirectoryGatherMessage, u64>,
    /// The output channels where the results are send to.
    #[allow(clippy::type_complexity)]
    output_channels: Vec<(
        Sender<InventoryEntryMessage>,
        Arc<Receiver<InventoryEntryMessage>>,
    )>,

    /// Sending an initial directory requires an stash.
    // PLANNED: Also used when one wants to push multiple directories.
    kickoff_stash: Mutex<GathererStash<'static>>,

    /// The maximum number of file descriptors this Gatherer may use.
    fd_limit: usize,

    /// LruList for keeping dir handles alive.
    dirs_lru:  LruList<Dir>,
    lru_batch: usize,

    /// Number of DirectoryGather Messages batched together
    message_batch: usize,
}

impl GathererInner {
    /// put a DirectoryGatherMessage on the input queue (traverse sub directories).
    pub(crate) fn send_dir(
        &self,
        message: DirectoryGatherMessage,
        prio: u64,
        stash: Option<&GathererStash>,
    ) {
        if let Some(stash) = stash {
            self.dirs_queue
                .send_batched(message, prio, self.message_batch, stash);
        } else {
            self.dirs_queue.send_nostash(message, prio);
        }
    }

    // resend a dir after 5ms pause to recover from filehandle depletion
    fn resend_dir(&self, message: DirectoryGatherMessage, prio: u64, stash: &GathererStash) {
        self.send_dir(message, prio, Some(stash));
        thread::sleep(std::time::Duration::from_millis(5));
    }

    /// Put a message on an output channel. The channels are used modulo the
    /// output_channels.len(), thus can never overflow and a user may use a hash/larger number
    /// than available.
    pub(crate) fn send_entry(&self, channel: usize, message: InventoryEntryMessage) {
        // Ignore result, the user may have dropped the receiver, but there is nothing we
        // should do about it.
        let _ = unsafe {
            self.output_channels
                .get_unchecked(channel % self.output_channels.len())
                .0
                .send(message)
        };
    }
}

/// Configures a Gatherer
pub struct GathererBuilder {
    num_gather_threads:  usize,
    num_output_channels: usize,
    inventory_backlog:   usize,
    fd_limit:            usize,
    lru_batch:           usize,
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
            lru_batch:           4,
            message_batch:       512,
        }
    }

    /// Starts the Gatherer. Takes the user defined processing function as argument. This
    /// function is used to process every directory entry seen. It should be small and fast
    /// selecting which sub directories to be traversed and which entries to pass to the
    /// output channels. Any more work should be done on the output then.
    pub fn start(&self, processor: Box<ProcessFn>) -> io::Result<Gatherer> {
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

        let gatherer = Gatherer(Arc::new(GathererInner {
            names: InternedNames::new(),
            dirs_queue: PriorityQueue::new(),
            output_channels,
            processor,
            kickoff_stash: Mutex::new(GathererStash::new_without_priority_queue()),
            fd_limit: self.fd_limit,
            dirs_lru: LruList::new(),
            lru_batch: self.lru_batch,
            message_batch: self.message_batch,
        }));

        (0..self.num_gather_threads).try_for_each(|n| -> io::Result<()> {
            gatherer.spawn_gather_thread(n)?;
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

    /// Sets the number of threads which traverse the directories. These are IO-bound
    /// operations and the more threads are used the better are the opportunities for the
    /// kernel to optimize IO-Requests. Tests have shown that on fast SSD's and cached data
    /// thread numbers in the hundrededs still show some benefits (at high resource
    /// costs). For general operation and on slower HDD's / non cached data 8-64 threads
    /// should be good enough. Default is 16 threads.
    #[must_use = "GathererBuilder must be used, call .start()"]
    pub fn with_output_channels(mut self, num_channels: usize) -> Self {
        assert!(num_channels > 0, "Must at least use one channel");
        self.num_output_channels = num_channels;
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

    /// Sets size of dir lru expires batched together.
    #[must_use = "GathererBuilder must be used, call .start()"]
    pub fn with_lru_batch(mut self, lru_batch: usize) -> Self {
        self.lru_batch = lru_batch;
        self
    }
}

#[cfg(test)]
mod test {
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    // tests
    #[test]
    fn smoke() {
        crate::test::init_env_logging();
        let _ = Gatherer::build().with_gather_threads(1).start(Box::new(
            |_gatherer: GathererHandle, _entry: ProcessMessage| {},
        ));
    }

    #[test]
    fn dir_smoke() {
        crate::test::init_env_logging();

        let gatherer = Gatherer::build()
            .with_gather_threads(4)
            .start(Box::new(
                |gatherer: GathererHandle, entry: ProcessMessage| match entry {
                    ProcessMessage::Result(Ok(entry), parent_path) => match entry.simple_type() {
                        Some(openat::SimpleType::Dir) => {
                            trace!("{:?}/{:?}", parent_path.to_pathbuf(), entry.file_name());
                            gatherer.traverse_dir(&entry, parent_path.clone(), false);
                        }
                        _ => {
                            trace!("{:?}/{:?}", parent_path.to_pathbuf(), entry.file_name());
                            gatherer.output_entry(0, &entry, parent_path, false);
                        }
                    },
                    ProcessMessage::Result(Err(err), parent_path) => {
                        gatherer.output_error(0, Box::new(err), parent_path);
                    }
                    _ => {}
                },
            ))
            .unwrap();

        gatherer.load_dir_recursive("src", false);

        gatherer
            .channel(0)
            .iter()
            .take_while(|msg| !matches!(msg, InventoryEntryMessage::Done))
            .for_each(|msg| {
                if let Some(path) = msg.path() {
                    trace!("{:?}", path.to_pathbuf());
                } else if msg.is_error() {
                    error!("{:?}", msg)
                }
            });
    }

    #[test]
    #[ignore]
    fn load_dir() {
        crate::test::init_env_logging();

        let gatherer = Gatherer::build()
            .with_gather_threads(64)
            .with_fd_limit(768)
            .start(Box::new(
                |gatherer: GathererHandle, entry: ProcessMessage| match entry {
                    ProcessMessage::Result(Ok(entry), parent_path) => match entry.simple_type() {
                        Some(openat::SimpleType::Dir) => {
                            gatherer.traverse_dir(&entry, parent_path.clone(), false);
                            gatherer.output_entry(0, &entry, parent_path.clone(), false);
                        }
                        _ => {
                            gatherer.output_entry(0, &entry, parent_path, false);
                        }
                    },
                    ProcessMessage::Result(Err(err), parent_path) => {
                        gatherer.output_error(0, Box::new(err), parent_path);
                    }
                    _ => {}
                },
            ))
            .unwrap();

        gatherer.load_dir_recursive(".", false);

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
                |gatherer: GathererHandle, entry: ProcessMessage| match entry {
                    ProcessMessage::Result(Ok(entry), parent_path) => match entry.simple_type() {
                        Some(openat::SimpleType::Dir) => {
                            gatherer.traverse_dir(&entry, parent_path, false);
                        }
                        _ => {
                            gatherer.output_entry(0, &entry, parent_path, false);
                        }
                    },
                    ProcessMessage::Result(Err(err), parent_path) => {
                        gatherer.output_error(0, Box::new(err), parent_path);
                    }
                    _ => {}
                },
            ))
            .unwrap();

        gatherer.load_dir_recursive("src", false);

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
                |gatherer: GathererHandle, entry: ProcessMessage| match entry {
                    ProcessMessage::Result(Ok(entry), parent_path) => match entry.simple_type() {
                        Some(openat::SimpleType::Dir) => {
                            gatherer.traverse_dir(&entry, parent_path.clone(), false);
                        }
                        _ => match parent_path.dir().unwrap().metadata(entry.file_name()) {
                            Ok(metadata) => {
                                gatherer.output_metadata(0, &entry, parent_path, metadata, false);
                            }
                            Err(err) => {
                                gatherer.output_error(0, Box::new(err), parent_path);
                            }
                        },
                    },
                    ProcessMessage::Result(Err(err), parent_path) => {
                        gatherer.output_error(0, Box::new(err), parent_path);
                    }
                    _ => {}
                },
            ))
            .unwrap();
        gatherer.load_dir_recursive("src", false);

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
    #[ignore]
    fn done_notifier() {
        crate::test::init_env_logging();

        let gatherer = Gatherer::build()
            .start(Box::new(
                |gatherer: GathererHandle, entry: ProcessMessage| match entry {
                    ProcessMessage::Result(Ok(entry), parent_path) => match entry.simple_type() {
                        Some(openat::SimpleType::Dir) => {
                            gatherer.traverse_dir(&entry, parent_path, true);
                        }
                        _ => {
                            gatherer.output_entry(0, &entry, parent_path, true);
                        }
                    },
                    ProcessMessage::Result(Err(err), parent_path) => {
                        gatherer.output_error(0, Box::new(err), parent_path);
                    }
                    ProcessMessage::ObjectDone(path) => {
                        trace!("got notified: {:?}", path);
                        gatherer.output_object_done(0, path);
                    }
                    _ => {}
                },
            ))
            .unwrap();

        gatherer.load_dir_recursive("src", true);

        gatherer.channel(0).iter().for_each(|msg| {
            trace!("output: {:?}", msg);
        });
    }
}
