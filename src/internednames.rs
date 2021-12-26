use std::path::Path;
use std::ffi::{OsStr, OsString};
use std::sync::Arc;
use std::sync::Mutex;
use std::borrow::Borrow;
use std::ops::Deref;
use std::collections::HashSet;

/// Storage for all interned names.
pub struct InternedNames {
    cached_names: Mutex<HashSet<InternedName>>,
}

impl InternedNames {
    /// Create a new InternedNames storage
    pub fn new() -> InternedNames {
        InternedNames {
            cached_names: Mutex::new(HashSet::new()),
        }
    }

    /// interns the given name from a reference by either creating a new instance or
    /// returning a reference to the existing instance.
    pub fn interning(&self, name: &OsStr) -> InternedName {
        self.cached_names
            .lock()
            .unwrap()
            .get_or_insert_with(name, InternedName::new)
            .clone()
    }

    // PLANNED: remove all entries with refcount == 1 (drain_filter) from cached_names
    // fn garbage_collect() {
    // }
}

impl Default for InternedNames {
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
