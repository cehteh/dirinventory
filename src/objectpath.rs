use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use std::sync::Arc;
use std::sync::atomic::{self, AtomicBool};

#[allow(unused_imports)]
pub use log::{debug, error, info, trace, warn};
use derivative::Derivative;

use crate::InternedName;

/// Space efficient storage of paths. Instead storing full path-names it stores only interned
/// strings of the actual object names and a reference to its parent. ObjectPaths are reference counted.
#[derive(Derivative)]
#[derivative(Hash, PartialOrd, PartialEq, Ord, Eq)]
pub struct ObjectPath(Arc<ObjectPathInner>);

impl ObjectPath {
    /// Creates a new ObjectPath without a parent. The user must supply 'watched = true' when
    /// this object shall generate a notification when it becomes dropped.
    pub fn new_object<P: AsRef<Path>>(path: P, watched: bool) -> ObjectPath {
        ObjectPath(Arc::new(ObjectPathInner {
            parent:  None,
            name:    InternedName::new(path.as_ref().as_os_str()),
            watched: AtomicBool::new(watched),
        }))
    }

    /// Creates a new ObjectPath without a parent and watching enabled.
    pub fn new<P: AsRef<Path>>(path: P) -> ObjectPath {
        ObjectPath::new_object(path, true)
    }

    /// Creates a new ObjectPath as sub-object to some existing ObjectPath object.  The user
    /// must supply 'watched = true' when this object shall generate a notification when it
    /// becomes dropped.
    #[must_use]
    pub fn sub_object(&self, name: InternedName, watched: bool) -> ObjectPath {
        ObjectPath(Arc::new(ObjectPathInner {
            parent: Some(self.clone()),
            name,
            watched: AtomicBool::new(watched),
        }))
    }

    /// Creates a new ObjectPath as sub object to some existing ObjectPath object with
    /// watching enabled.
    #[must_use]
    pub fn sub_directory(&self, name: InternedName) -> ObjectPath {
        self.sub_object(name, true)
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
}

impl Clone for ObjectPath {
    fn clone(&self) -> Self {
        ObjectPath(self.0.clone())
    }
}

impl Drop for ObjectPath {
    fn drop(&mut self) {
        if self.strong_count() == 1 && self.0.watched.fetch_and(false, atomic::Ordering::Relaxed) {
            // todo!("Call callback");
            trace!("Drop {:?}", self.to_pathbuf())
        }
    }
}
#[derive(Derivative)]
#[derivative(Hash, PartialOrd, PartialEq, Ord, Eq)]
pub struct ObjectPathInner {
    parent:  Option<ObjectPath>,
    name:    InternedName,
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

#[test]
fn objectpath_path_smoke() {
    assert_eq!(ObjectPath::new(".").to_pathbuf(), PathBuf::from("."));
}

#[test]
fn objectpath_path_subobject() {
    use std::ffi::OsStr;
    let p = ObjectPath::new(".");
    let mut pathbuf = PathBuf::new();
    assert_eq!(
        p.sub_directory(InternedName::new(OsStr::new("foo")))
            .write_pathbuf(&mut pathbuf),
        &PathBuf::from("./foo")
    );
}

#[test]
fn objectpath_path_ordering() {
    let foo = ObjectPath::new("foo");
    let bar = ObjectPath::new("bar");
    assert!(bar < foo);

    let bar2 = ObjectPath::new("bar");
    assert!(bar == bar2);

    let foobar = foo.sub_directory(InternedName::new(OsStr::new("bar")));
    let barfoo = bar.sub_directory(InternedName::new(OsStr::new("foo")));
    assert!(barfoo < foobar);
}

#[test]
fn objectpath_metadata() {
    let cargo = ObjectPath::new("Cargo.toml");
    assert!(cargo.metadata().is_ok());
}
