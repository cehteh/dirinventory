#[allow(unused_imports)]
use crate::{debug, error, info, trace, warn};
use crate::*;

/// Defines the API the user defined ProcessFn may use to send data back on the
/// input queue and output channels.
pub struct GathererHandle<'a> {
    gatherer: &'a Gatherer,
    stash:    Option<&'a GathererStash<'a>>,
}

impl GathererHandle<'_> {
    pub(crate) fn new<'a>(
        gatherer: &'a Gatherer,
        stash: Option<&'a GathererStash>,
    ) -> GathererHandle<'a> {
        GathererHandle { gatherer, stash }
    }

    /// Add a (sub-) directory to the input priority queue to be traversed as well.
    /// This must be a directory, otherwise it panics.y
    pub fn traverse_dir(&self, entry: &openat::Entry, parent_path: ObjectPath, watched: bool) {
        assert!(matches!(entry.simple_type(), Some(openat::SimpleType::Dir)));
        let subdir = parent_path.sub_object(entry.file_name(), self.gatherer);

        subdir.watch(watched);
        // FIXME: retain parent_path's dir? may need ObjectPath::adjust() to change Arc/Weak
        // state based on watch flag when done with parent dir

        // The Order of directory traversal is defined by the 64bit priority in the
        // PriorityQueue. This 64bit are composed of the inode number added directory
        // depth inversed from u64::MAX down shifted by 48 bits (resulting in the
        // upper 16bits for the priority). This results in that directories are
        // traversed depth first in inode increasing order.
        // PLANNED: When deeper than 64k consider it as loop? do a explicit loop check?
        let dir_prio = ((u16::MAX - subdir.depth()) as u64) << 48;
        let message = DirectoryGatherMessage::new_dir(subdir);

        self.inner()
            .send_dir(message, dir_prio + entry.inode(), self.stash);
    }

    /// Sends openat::Entry components to the output channel. 'channel' can be any number as send wraps
    /// it by modulo the real number of channels. This allows to use any usize hash or
    /// otherwise large number.
    pub fn output_entry(
        &self,
        channel: usize,
        entry: &openat::Entry,
        parent_path: ObjectPath,
        watched: bool,
    ) {
        let path = ObjectPath::sub_object(&parent_path, entry.file_name(), self.gatherer);

        path.watch(watched);

        self.inner()
            .send_entry(channel, InventoryEntryMessage::Entry {
                path,
                file_type: entry.simple_type(),
                inode: entry.inode(),
            });
    }

    /// Sends openat::Metadata to the output channel.  'channel' can be any number as send wraps
    /// it by modulo the actual number of channels. This allows to use any usize hash or
    /// otherwise large number.
    pub fn output_metadata(
        &self,
        channel: usize,
        entry: &openat::Entry,
        parent_path: ObjectPath,
        metadata: openat::Metadata,
        watched: bool,
    ) {
        let entryname = ObjectPath::sub_object(&parent_path, entry.file_name(), self.gatherer);

        entryname.watch(watched);

        self.inner()
            .send_entry(channel, InventoryEntryMessage::Metadata {
                path: entryname,
                metadata,
            });
    }

    /// Sends the ObjectDone notificaton to the output channel.  'channel' can be any number
    /// as send wraps it by modulo the actual number of channels. This allows to use any usize
    /// hash or otherwise large number.
    pub fn output_object_done(&self, channel: usize, path: ObjectPath) {
        self.inner()
            .send_entry(channel, InventoryEntryMessage::ObjectDone { path });
    }

    /// Sends an error to the output channel.  'channel' can be any number as send wraps
    /// it by modulo the real number of channels. This allows to use any usize hash or
    /// otherwise large number.
    pub fn output_error(&self, channel: usize, error: DynError, path: ObjectPath) {
        // FIXME: make sure the offending path is passed by each caller
        warn!("{:?} at {:?}", error, path);
        self.inner()
            .send_entry(channel, InventoryEntryMessage::Err { path, error });
    }

    fn inner(&self) -> &GathererInner {
        self.gatherer.inner()
    }
}
