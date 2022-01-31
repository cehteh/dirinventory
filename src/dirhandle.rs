//! Wraps descriptors, adds global accounting etc.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::io;

use openat_ct as openat;

#[allow(unused_imports)]
use crate::{debug, error, info, trace, warn};

static USED_HANDLES: AtomicUsize = AtomicUsize::new(0);

/// Return the number of file handles currently in use by Dir.
pub fn used_handles() -> usize {
    USED_HANDLES.load(Ordering::Relaxed)
}

// danger! only used when a dir iterator is done
pub(crate) fn dec_handles() -> usize {
    USED_HANDLES.fetch_sub(1, Ordering::Relaxed)
}

/// Wraps openat::Dir adds counting of used fd's
#[derive(Debug)]
pub struct Dir(openat::Dir);

impl Dir {
    /// see openat::open()
    pub fn open<P: openat::AsPath + std::fmt::Debug>(path: P) -> io::Result<Dir> {
        let dir = openat::Dir::flags().with(openat::O_DIRECTORY).open(path)?;
        USED_HANDLES.fetch_add(1, Ordering::Relaxed);
        Ok(Dir(dir))
    }

    /// see openat::sub_dir()
    pub fn sub_dir<P: openat::AsPath + std::fmt::Debug>(&self, path: P) -> io::Result<Dir> {
        let dir = self.0.with(openat::O_DIRECTORY).sub_dir(path)?;
        USED_HANDLES.fetch_add(1, Ordering::Relaxed);
        Ok(Dir(dir))
    }

    /// see openat::list_self()
    pub fn list_self(&self) -> io::Result<openat::DirIter> {
        let dir_iter = self.0.list_self()?;
        USED_HANDLES.fetch_add(1, Ordering::Relaxed);
        Ok(dir_iter)
    }

    /// see openat::metadata()
    pub fn metadata<P: openat::AsPath>(&self, path: P) -> io::Result<openat::Metadata> {
        self.0.metadata(path)
    }
}

/// Drop decrements the handle count
impl Drop for Dir {
    fn drop(&mut self) {
        USED_HANDLES.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn smoke() {
        crate::test::init_env_logging();
        Dir::open(".").unwrap();
    }
}
