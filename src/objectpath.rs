use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use std::sync::{Arc, Weak};
use std::sync::atomic::{self, AtomicBool};
use std::io;

use parking_lot::Mutex;
use derivative::Derivative;

#[allow(unused_imports)]
use crate::{debug, error, info, trace, warn};
use crate::{Dir, Gatherer, GathererInner, InternedName};

/// Space efficient storage of paths. Instead storing full path-names it stores only interned
/// strings of the actual object names and a reference to its parent. ObjectPaths are reference counted.
#[derive(Derivative)]
#[derivative(Hash, PartialOrd, PartialEq, Ord, Eq)]
pub struct ObjectPath(Arc<ObjectPathInner>);

impl ObjectPath {
    /// Creates a new ObjectPath without a parent.
    // pub fn new<P: AsRef<Path>>(path: P, gatherer: &Gatherer) -> ObjectPath {
    pub fn new(path: impl AsRef<Path>, gatherer: &Gatherer) -> ObjectPath {
        ObjectPath(Arc::new(ObjectPathInner {
            parent:   None,
            name:     gatherer.name_interning(path.as_ref().as_os_str()),
            dir:      Mutex::new(None),
            gatherer: gatherer.downgrade(),
            watched:  AtomicBool::new(false),
        }))
    }

    /// Creates a new ObjectPath without a parent and associated gatherer.
    pub fn without_gatherer<P: AsRef<Path>>(path: P) -> ObjectPath {
        ObjectPath(Arc::new(ObjectPathInner {
            parent:   None,
            name:     InternedName::new(path.as_ref().as_os_str()),
            dir:      Mutex::new(None),
            gatherer: Weak::new(),
            watched:  AtomicBool::new(false),
        }))
    }

    /// Creates a new ObjectPath as sub-object to some existing ObjectPath object.
    #[must_use]
    pub fn sub_object<P: AsRef<Path>>(&self, name: P, gatherer: &Gatherer) -> ObjectPath {
        ObjectPath(Arc::new(ObjectPathInner {
            parent:   Some(self.clone()),
            name:     gatherer.name_interning(name.as_ref().as_os_str()),
            dir:      Mutex::new(None),
            gatherer: gatherer.downgrade(),
            watched:  AtomicBool::new(false),
        }))
    }

    /// Creates a new ObjectPath as sub-object to some existing ObjectPath object without
    /// associated gatherer.
    #[must_use]
    pub fn sub_object_without_gatherer<P: AsRef<Path>>(&self, name: P) -> ObjectPath {
        ObjectPath(Arc::new(ObjectPathInner {
            parent:   Some(self.clone()),
            name:     InternedName::new(name.as_ref().as_os_str()),
            dir:      Mutex::new(None),
            gatherer: Weak::new(),
            watched:  AtomicBool::new(false),
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
        // FIXME: use parents dir handle if available
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

    /// Sets the notification state on this ObjectPath. When true and all processing handles get
    /// dropped a notification is issued. Watching an ObjectPath also retains its dir handle.
    pub fn watch(&self, watch: bool) {
        self.0.watched.store(watch, atomic::Ordering::Relaxed);
        // self.0.dir.lru_preserve();
    }

    /// Queries the notification state on this ObjectPath. When true and all processing handles get
    /// dropped a notification is issued.
    pub fn is_watched(&self) -> bool {
        self.0.watched.load(atomic::Ordering::Relaxed)
    }

    /// Gets a dir handle for this object.
    pub fn dir(&self) -> io::Result<Arc<Dir>> {
        let mut locked_dir = self.0.dir.lock();

        Ok(match locked_dir.as_ref().map(Weak::upgrade) {
            Some(Some(arc)) => {
                trace!("OPEN reuse handle {:?}", self);
                arc
            }
            _ => {
                let dir_arc = Arc::new(
                    match self
                        .parent()
                        .map(|parent| parent.0.dir.lock().as_ref().map(Weak::upgrade))
                    {
                        Some(Some(Some(parent_handle))) => {
                            trace!("OPEN subdir {:?}", self);
                            parent_handle.sub_dir(self.name())?
                        }
                        _ => {
                            trace!("OPEN new {:?}", self);
                            Dir::open(&self.to_pathbuf())?
                        }
                    },
                );
                *locked_dir = Some(Arc::downgrade(&dir_arc));

                dir_arc
            }
        })
    }

    /// Returns an Arc handle to the Gatherer if available
    pub fn gatherer(&self) -> Option<Gatherer> {
        Gatherer::upgrade(&self.0.gatherer)
    }

    /// Returns the parent if available
    pub fn parent(&self) -> Option<&ObjectPath> {
        self.0.parent.as_ref()
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

    #[derivative(
        Hash = "ignore",
        PartialEq = "ignore",
        PartialOrd = "ignore",
        Ord = "ignore"
    )]
    dir: Mutex<Option<Weak<Dir>>>,

    #[derivative(
        Hash = "ignore",
        PartialEq = "ignore",
        PartialOrd = "ignore",
        Ord = "ignore"
    )]
    gatherer: Weak<GathererInner>,

    #[derivative(
        Hash = "ignore",
        PartialEq = "ignore",
        PartialOrd = "ignore",
        Ord = "ignore"
    )]
    watched: AtomicBool,
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

    #[allow(unused_imports)]
    use crate::{debug, error, info, trace, warn};
    use crate::InternedName;
    use super::ObjectPath;

    #[test]
    fn smoke() {
        crate::test::init_env_logging();
        assert_eq!(
            ObjectPath::without_gatherer(".").to_pathbuf(),
            PathBuf::from(".")
        );
    }

    #[test]
    fn path_subobject() {
        crate::test::init_env_logging();
        use std::ffi::OsStr;
        let p = ObjectPath::without_gatherer(".");
        let mut pathbuf = PathBuf::new();
        assert_eq!(
            p.sub_object_without_gatherer(InternedName::new(OsStr::new("foo")))
                .write_pathbuf(&mut pathbuf),
            &PathBuf::from("./foo")
        );
    }

    #[test]
    fn path_ordering() {
        crate::test::init_env_logging();
        let foo = ObjectPath::without_gatherer("foo");
        let bar = ObjectPath::without_gatherer("bar");
        assert!(bar < foo);

        let bar2 = ObjectPath::without_gatherer("bar");
        assert!(bar == bar2);

        let foobar = foo.sub_object_without_gatherer(InternedName::new(OsStr::new("bar")));
        let barfoo = bar.sub_object_without_gatherer(InternedName::new(OsStr::new("foo")));
        assert!(barfoo < foobar);
    }

    #[test]
    fn metadata() {
        crate::test::init_env_logging();
        let cargo = ObjectPath::without_gatherer("Cargo.toml");
        assert!(cargo.metadata().is_ok());
    }
}
