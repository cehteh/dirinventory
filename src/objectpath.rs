use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use std::sync::{Arc, Weak};
use std::sync::atomic::{self, AtomicBool};

#[allow(unused_imports)]
pub use log::{debug, error, info, trace, warn};
use derivative::Derivative;

use crate::{Gatherer, InternedName};

/// Space efficient storage of paths. Instead storing full path-names it stores only interned
/// strings of the actual object names and a reference to its parent. ObjectPaths are reference counted.
#[derive(Derivative)]
#[derivative(Hash, PartialOrd, PartialEq, Ord, Eq)]
pub struct ObjectPath(Arc<ObjectPathInner>);

impl ObjectPath {
    /// Creates a new ObjectPath without a parent.
    pub fn new<P: AsRef<Path>>(path: P, gatherer: Weak<Gatherer>) -> ObjectPath {
        ObjectPath(Arc::new(ObjectPathInner {
            parent: None,
            name: InternedName::new(path.as_ref().as_os_str()),
            gatherer,
            watched: AtomicBool::new(false),
        }))
    }

    /// Creates a new ObjectPath as sub-object to some existing ObjectPath object.  The user
    /// must supply 'watched = true' when this object shall generate a notification when it
    /// becomes dropped.
    #[must_use]
    pub fn sub_object(&self, name: InternedName, gatherer: Weak<Gatherer>) -> ObjectPath {
        ObjectPath(Arc::new(ObjectPathInner {
            parent: Some(self.clone()),
            name,
            gatherer,
            watched: AtomicBool::new(false),
        }))
    }

    fn pathbuf_push_parents(&self, target: &mut PathBuf, len: usize) {
        if let Some(parent) = &self.0.parent {
            parent.pathbuf_push_parents(
                target,
                len + self.0.name.len() + 1, // delimiter char
            )
        } else {
            target.reserve(len + self.0.name.len());
        };
        target.push(&*self.0.name);
    }

    /// Writes the full ObjectPath including all parents to the given PathBuf.
    pub fn write_pathbuf<'a>(&self, target: &'a mut PathBuf) -> &'a PathBuf {
        target.clear();
        self.pathbuf_push_parents(target, 1 /* for root delimiter */);
        target
    }

    /// Create a new PathBuf from the given ObjectPath.
    pub fn to_pathbuf(&self) -> PathBuf {
        // TODO: iterative impl
        let mut target = PathBuf::new();
        self.pathbuf_push_parents(&mut target, 1 /* for root delimiter */);
        target
    }

    // Returns path length in bytes including delimiters.
    // pub fn len(&self) -> usize {
    //
    // }

    /// Returns the number of components in the path.
    pub fn depth(&self) -> u16 {
        let mut counter = 1u16;
        let mut itr = &self.0;
        while let Some(parent) = &itr.parent {
            itr = &parent.0;
            counter += 1;
        }
        counter
    }

    /// Returns an reference to the name of the object, without any preceding path components.
    pub fn name(&self) -> &OsStr {
        &self.0.name
    }

    /// Return the metadata of an objectpath
    pub fn metadata(&self) -> std::io::Result<crate::openat::Metadata> {
        let parent = if let Some(parent) = &self.0.parent {
            parent.to_pathbuf()
        } else {
            PathBuf::from(if Path::new(&*self.0.name).is_absolute() {
                std::path::Component::RootDir.as_os_str()
            } else {
                std::path::Component::CurDir.as_os_str()
            })
        };

        crate::openat::Dir::open(&parent)?.metadata(&*self.0.name)
    }

    /// Returns the number of strong references pointing to this object
    pub fn strong_count(&self) -> usize {
        Arc::strong_count(&self.0)
    }

    /// Enables watching on this ObjectPath, whenever all processing handles get dropped a
    /// notification is issued.
    pub fn watch(&self) {
        self.0.watched.store(true, atomic::Ordering::Relaxed);
    }

    /// Returns am Arc handle to the Gatherer if available
    pub fn gatherer(&self) -> Option<Arc<Gatherer>> {
        self.0.gatherer.upgrade()
    }
}

impl Clone for ObjectPath {
    fn clone(&self) -> Self {
        ObjectPath(self.0.clone())
    }
}

impl Drop for ObjectPath {
    fn drop(&mut self) {
        // Ordering::Relaxed suffices here because the last reference is always the gatherer
        // itself (which starts with two references). The gatherer will never increase the
        // refcount when it becomes dropped to one. Disable watching, so that notify() can
        // clone the path without retriggering the notifier. 2 because this is the refcount
        // before this drop happend.
        if self.strong_count() <= 2 && self.0.watched.swap(false, atomic::Ordering::Relaxed) {
            if let Some(gatherer) = self.gatherer() {
                gatherer.notify_path_dropped(self.clone());
            }
        }
    }
}

#[derive(Derivative)]
#[derivative(Hash, PartialOrd, PartialEq, Ord, Eq)]
pub struct ObjectPathInner {
    parent: Option<ObjectPath>,
    name:   InternedName,

    // PLANNED: include enum MaybeHandle Weak/Strong/None Weak(Weak<Dir>) ...
    #[derivative(
        Hash = "ignore",
        PartialEq = "ignore",
        PartialOrd = "ignore",
        Ord = "ignore"
    )]
    gatherer: Weak<Gatherer>,
    #[derivative(
        Hash = "ignore",
        PartialEq = "ignore",
        PartialOrd = "ignore",
        Ord = "ignore"
    )]
    watched:  AtomicBool,
}

use std::fmt;
impl fmt::Debug for ObjectPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_pathbuf())
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;
    use std::ffi::OsStr;
    use std::sync::Weak;

    #[allow(unused_imports)]
    pub use log::{debug, error, info, trace, warn};

    use crate::InternedName;
    use super::ObjectPath;

    #[test]
    fn smoke() {
        crate::test::init_env_logging();
        assert_eq!(
            ObjectPath::new(".", Weak::new()).to_pathbuf(),
            PathBuf::from(".")
        );
    }

    #[test]
    fn path_subobject() {
        crate::test::init_env_logging();
        use std::ffi::OsStr;
        let p = ObjectPath::new(".", Weak::new());
        let mut pathbuf = PathBuf::new();
        assert_eq!(
            p.sub_object(InternedName::new(OsStr::new("foo")), Weak::new())
                .write_pathbuf(&mut pathbuf),
            &PathBuf::from("./foo")
        );
    }

    #[test]
    fn path_ordering() {
        crate::test::init_env_logging();
        let foo = ObjectPath::new("foo", Weak::new());
        let bar = ObjectPath::new("bar", Weak::new());
        assert!(bar < foo);

        let bar2 = ObjectPath::new("bar", Weak::new());
        assert!(bar == bar2);

        let foobar = foo.sub_object(InternedName::new(OsStr::new("bar")), Weak::new());
        let barfoo = bar.sub_object(InternedName::new(OsStr::new("foo")), Weak::new());
        assert!(barfoo < foobar);
    }

    #[test]
    fn metadata() {
        crate::test::init_env_logging();
        let cargo = ObjectPath::new("Cargo.toml", Weak::new());
        assert!(cargo.metadata().is_ok());
    }
}
