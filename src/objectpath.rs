use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use std::sync::Arc;

use crate::InternedName;

/// Space efficient storage of paths. Instead storing full path-names it stores only interned
/// strings of the actual object names and a reference to its parent. Note tat since parents
/// are usually shared between all ObjectPath instances, the API uses Arc<ObjectPath> instead
/// plain objects.
#[derive(Hash, PartialOrd, PartialEq, Ord)]
pub struct ObjectPath {
    parent: Option<Arc<ObjectPath>>,
    name:   InternedName,
}

impl Eq for ObjectPath {}

impl ObjectPath {
    /// Creates a new ObjectPath without a parent.
    pub fn new<P: AsRef<Path>>(path: P) -> Arc<ObjectPath> {
        Arc::new(ObjectPath {
            parent: None,
            name:   InternedName::new(path.as_ref().as_os_str()),
        })
    }

    /// Creates a new ObjectPath as sub-object to some existing ObjectPath object.
    pub fn subobject(self: Arc<Self>, name: InternedName) -> Arc<ObjectPath> {
        Arc::new(ObjectPath {
            parent: Some(self),
            name,
        })
    }

    fn pathbuf_push_parents(&self, target: &mut PathBuf, len: usize) {
        if let Some(parent) = &self.parent {
            parent.pathbuf_push_parents(target, len + self.name.len() + 1 /* delimiter char */)
        } else {
            target.reserve(len + self.name.len());
        };
        target.push(&*self.name);
    }

    /// Writes the full ObjectPath to the given PathBuf.
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
        let mut itr = self;
        while let Some(parent) = &itr.parent {
            itr = parent;
            counter += 1;
        }
        counter
    }

    /// Returns an reference to the name of the object, without any preceding path components.
    pub fn name(&self) -> &OsStr {
        &self.name
    }

    /// Return the metadata of an objectpath
    pub fn metadata(&self) -> std::io::Result<crate::openat::Metadata> {
        let parent = if let Some(parent) = &self.parent {
            parent.to_pathbuf()
        } else {
            PathBuf::from(if Path::new(&*self.name).is_absolute() {
                std::path::Component::RootDir.as_os_str()
            } else {
                std::path::Component::CurDir.as_os_str()
            })
        };

        crate::openat::Dir::open(&parent)?.metadata(&*self.name)
    }
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
        p.subobject(InternedName::new(OsStr::new("foo")))
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

    let foobar = foo.clone().subobject(InternedName::new(OsStr::new("bar")));
    let barfoo = bar.clone().subobject(InternedName::new(OsStr::new("foo")));
    assert!(barfoo < foobar);
}

#[test]
fn objectpath_metadata() {
    let cargo = ObjectPath::new("Cargo.toml");
    assert!(cargo.metadata().is_ok());
}
