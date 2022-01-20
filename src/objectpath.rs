use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use std::sync::Arc;
use std::sync::atomic::{self, AtomicBool};
use std::marker::PhantomData;

#[allow(unused_imports)]
pub use log::{debug, error, info, trace, warn};
use derivative::Derivative;

use crate::InternedName;

/// Space efficient storage of paths. Instead storing full path-names it stores only interned
/// strings of the actual object names and a reference to its parent. ObjectPaths are reference counted.
#[derive(Derivative)]
#[derivative(Hash, PartialOrd, PartialEq, Ord, Eq)]
pub struct ObjectPath<D: DropNotify>(Arc<ObjectPathInner<D>>, PhantomData<D>);

impl<D: DropNotify> ObjectPath<D> {
    /// Creates a new ObjectPath without a parent. The user must supply 'watched = true' when
    /// this object shall generate a notification when it becomes dropped.
    pub fn new_object<P: AsRef<Path>>(path: P, watched: bool) -> ObjectPath<D> {
        ObjectPath(
            Arc::new(ObjectPathInner {
                parent:  None,
                name:    InternedName::new(path.as_ref().as_os_str()),
                watched: AtomicBool::new(watched),
            }),
            PhantomData,
        )
    }

    /// Creates a new ObjectPath without a parent and watching enabled.
    pub fn new<P: AsRef<Path>>(path: P) -> ObjectPath<D> {
        ObjectPath::new_object(path, true)
    }

    /// Creates a new ObjectPath as sub-object to some existing ObjectPath object.  The user
    /// must supply 'watched = true' when this object shall generate a notification when it
    /// becomes dropped.
    #[must_use]
    pub fn sub_object(&self, name: InternedName, watched: bool) -> ObjectPath<D> {
        ObjectPath(
            Arc::new(ObjectPathInner {
                parent: Some(self.clone()),
                name,
                watched: AtomicBool::new(watched),
            }),
            PhantomData,
        )
    }

    /// Creates a new ObjectPath as sub object to some existing ObjectPath object with
    /// watching enabled.
    #[must_use]
    pub fn sub_directory(&self, name: InternedName) -> ObjectPath<D> {
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

impl<D: DropNotify> Clone for ObjectPath<D> {
    fn clone(&self) -> Self {
        ObjectPath(self.0.clone(), PhantomData)
    }
}

impl<D: DropNotify> Drop for ObjectPath<D> {
    fn drop(&mut self) {
        // Ordering::Relaxed suffices here because the last reference is always the gatherer
        // itself (which starts with two references). The gatherer will never increase the
        // refcount when it becomes dropped to one. Disable watching, so that notify() can
        // clone the path without retriggering the notifier. 2 because this is the refcount
        // before this drop happend.
        if self.strong_count() <= 2 && self.0.watched.swap(false, atomic::Ordering::Relaxed) {
            D::notify(self);
        }
    }
}

pub trait DropNotify {
    fn notify<D: DropNotify>(path: &mut ObjectPath<D>);
}

impl DropNotify for () {
    fn notify<D: DropNotify>(_path: &mut ObjectPath<D>) {
        // nothing for <()>
    }
}

#[derive(Derivative)]
#[derivative(Hash, PartialOrd, PartialEq, Ord, Eq)]
pub struct ObjectPathInner<D: DropNotify> {
    parent:  Option<ObjectPath<D>>,
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
impl<D: DropNotify> fmt::Debug for ObjectPath<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_pathbuf())
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;
    use std::ffi::OsStr;
    use std::sync::{Mutex, MutexGuard};
    use std::lazy::SyncOnceCell;
    use std::sync::atomic;

    #[allow(unused_imports)]
    pub use log::{debug, error, info, trace, warn};

    use crate::InternedName;
    use super::{DropNotify, ObjectPath};

    #[test]
    fn smoke() {
        crate::test::init_env_logging();
        assert_eq!(ObjectPath::<()>::new(".").to_pathbuf(), PathBuf::from("."));
    }

    #[test]
    fn path_subobject() {
        crate::test::init_env_logging();
        use std::ffi::OsStr;
        let p = ObjectPath::<()>::new(".");
        let mut pathbuf = PathBuf::new();
        assert_eq!(
            p.sub_directory(InternedName::new(OsStr::new("foo")))
                .write_pathbuf(&mut pathbuf),
            &PathBuf::from("./foo")
        );
    }

    #[test]
    fn path_ordering() {
        crate::test::init_env_logging();
        let foo = ObjectPath::<()>::new("foo");
        let bar = ObjectPath::<()>::new("bar");
        assert!(bar < foo);

        let bar2 = ObjectPath::<()>::new("bar");
        assert!(bar == bar2);

        let foobar = foo.sub_directory(InternedName::new(OsStr::new("bar")));
        let barfoo = bar.sub_directory(InternedName::new(OsStr::new("foo")));
        assert!(barfoo < foobar);
    }

    #[test]
    fn metadata() {
        crate::test::init_env_logging();
        let cargo = ObjectPath::<()>::new("Cargo.toml");
        assert!(cargo.metadata().is_ok());
    }

    #[test]
    fn dropnotify_smoke() {
        crate::test::init_env_logging();
        let _cargo = ObjectPath::<()>::new("Cargo.toml");
    }

    macro_rules! impl_test_notifier {
        () => {
            struct TestNotifier;

            impl TestNotifier {
                fn data() -> MutexGuard<'static, Vec<PathBuf>> {
                    static LAST_SEEN: SyncOnceCell<Mutex<Vec<PathBuf>>> = SyncOnceCell::new();
                    LAST_SEEN
                        .get_or_init(|| Mutex::new(Vec::new()))
                        .lock()
                        .unwrap()
                }

                fn push(path: PathBuf) {
                    trace!("pushing dropped value: {:?}", path);
                    Self::data().push(path);
                }
            }

            impl DropNotify for TestNotifier {
                fn notify<D: DropNotify>(path: &mut ObjectPath<D>) {
                    let last = path.to_pathbuf();
                    trace!("dropped: {:?}", last);
                    TestNotifier::push(last);
                }
            }
        };
    }

    #[test]
    fn dropnotify_test() {
        impl_test_notifier!();
        crate::test::init_env_logging();
        let cargo = ObjectPath::<TestNotifier>::new("Cargo.toml");
        trace!("got: {:?}", cargo.to_pathbuf());
        drop(cargo);
        assert_eq!(
            TestNotifier::data().last(),
            Some(&PathBuf::from("Cargo.toml"))
        );
    }

    #[test]
    fn dropnotify_dropcount() {
        impl_test_notifier!();
        crate::test::init_env_logging();
        let cargo = ObjectPath::<TestNotifier>::new("Cargo.toml");
        let cargo2 = cargo.clone();
        let cargo3 = cargo.clone();

        assert_eq!(cargo.0.watched.load(atomic::Ordering::Relaxed), true);
        assert_eq!(cargo.strong_count(), 3);
        drop(cargo2);

        assert_eq!(cargo.0.watched.load(atomic::Ordering::Relaxed), true);
        assert_eq!(cargo.strong_count(), 2);
        assert_eq!(TestNotifier::data().len(), 0);
        drop(cargo3);

        assert_eq!(cargo.strong_count(), 1);
        assert_eq!(cargo.0.watched.load(atomic::Ordering::Relaxed), false);
        assert_eq!(TestNotifier::data().len(), 1);

        assert_eq!(
            TestNotifier::data().last(),
            Some(&PathBuf::from("Cargo.toml"))
        );
    }
}
