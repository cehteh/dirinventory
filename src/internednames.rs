use std::path::Path;
use std::ffi::{OsStr, OsString};
use std::sync::Arc;
use std::borrow::Borrow;
use std::ops::Deref;
use std::collections::HashSet;
use std::os::unix::ffi::OsStrExt;

use parking_lot::Mutex;

/// Storage for all interned names. using a sharded HashSet with N shards
pub struct InternedNames<const N: usize> {
    cached_names: [Mutex<HashSet<InternedName>>; N],
}

impl<const N: usize> InternedNames<N> {
    /// Create a new InternedNames storage
    pub fn new() -> InternedNames<N> {
        InternedNames {
            cached_names: [(); N].map(|()| Mutex::new(HashSet::new())),
        }
    }

    /// interns the given name from a reference by either creating a new instance or
    /// returning a reference to the existing instance.
    pub fn interning(&self, name: &OsStr) -> InternedName {
        self.cached_names[name.bucket::<N>()]
            .lock()
            .get_or_insert_with(name, InternedName::new)
            .clone()
    }

    // PLANNED: remove all entries with refcount == 1 (drain_filter) from cached_names
    // fn garbage_collect() {
    // }
}

impl<const N: usize> Default for InternedNames<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// An Arc shared OsString used for objectnames
#[derive(Debug, Hash, PartialOrd, PartialEq, Eq, Ord)]
pub struct InternedName(Arc<OsString>);

impl InternedName {
    /// Create a new InterenedName from an OsStr reference.  This InternedName is not yet part
    /// of any InternedNames collection. Use InternedNames::interning() for that!
    pub fn new(s: &OsStr) -> InternedName {
        InternedName(Arc::new(OsString::from(s)))
    }
}

impl Borrow<OsStr> for InternedName {
    fn borrow(&self) -> &OsStr {
        &self.0
    }
}

impl Deref for InternedName {
    type Target = OsStr;

    fn deref(&self) -> &OsStr {
        &self.0
    }
}

impl Clone for InternedName {
    fn clone(&self) -> InternedName {
        InternedName(self.0.clone())
    }
}

impl AsRef<Path> for InternedName {
    fn as_ref(&self) -> &Path {
        Path::new(&*self.0)
    }
}

/// Defines into which bucket a key falls.
trait Bucketize {
    fn bucket<const N: usize>(&self) -> usize;
}

/// OsStr specialization just sums up all characters modulo buckets
impl Bucketize for OsStr {
    fn bucket<const N: usize>(&self) -> usize {
        self.as_bytes()
            .iter()
            .map(|a| usize::from(*a))
            .sum::<usize>()
            % N
    }
}
