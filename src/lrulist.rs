use std::sync::Arc;
use std::fmt::{self, Debug, Formatter};

use crossbeam_channel::{unbounded, Receiver, Sender};

#[allow(unused_imports)]
use crate::{debug, error, info, trace, warn};

/// Keeping objects alive by passing Arc's through a channel. As long the object exists in the
/// channel it can be revived from a Weak pointer. Objects can appear multiply on this
/// channel, make sure to 'preserve' then only when neccessary.
pub(crate) struct LruList<T: Debug>(Sender<Arc<T>>, Receiver<Arc<T>>);

impl<T: Debug> LruList<T> {
    /// Create a new LruList.
    pub fn new() -> LruList<T> {
        let (s, r) = unbounded();
        LruList(s, r)
    }

    /// Pushes an Arc onto the LruList.
    pub fn preserve(&self, element: Arc<T>) {
        trace!("preserve {:?}: {:?}", self, element);
        let _ = self.0.send(element);
    }

    /// Expires up to 'n' elements. Only elements that are really preserved by the LruList are
    /// counting against 'n'. That is their single reference is here in this list.  Returns
    /// 'true' when 'n' elements got expired and false when there where less than 'n' elements
    /// expirable.
    pub fn expire(&self, mut n: usize) -> bool {
        while n >= 1 {
            match self.1.try_recv() {
                Ok(elem) if Arc::strong_count(&elem) == 1 => {
                    trace!("expire {:?}: {:?}", self, elem);
                    n -= 1;
                }
                Ok(elem) => {
                    trace!("drop {:?}: {:?}", self, elem);
                }
                Err(_) => {
                    trace!("nothing to expire {:?}", self);
                    return false;
                }
            }
        }
        true
    }

    /// Expires all elements in the LruList.
    pub fn expire_all(&self) {
        self.expire(usize::MAX);
    }

    /// Expires elements in batches of 'n' from a LruList until 'pedicate' returns true.
    /// Returns 'true' when enough elements to satisfy 'predicate' could be expired and
    /// 'false' when there where not enough elements to expire.
    pub fn expire_until(&self, batch: usize, predicate: Box<dyn Fn() -> bool>) -> bool {
        while !predicate() {
            if !self.expire(batch) {
                return false;
            }
        }
        true
    }
}

impl<T: Debug> Debug for LruList<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self as *const Self)
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::LruList;
    #[allow(unused_imports)]
    use crate::{debug, error, info, trace, warn};

    #[test]
    fn smoke() {
        crate::test::init_env_logging();

        let lrulist = LruList::<i32>::new();
        let a = Arc::new(42);
        lrulist.preserve(a.clone());
        assert_eq!(Arc::strong_count(&a), 2);
    }

    #[test]
    fn expire() {
        crate::test::init_env_logging();

        let lrulist = LruList::<i32>::new();
        let a = Arc::new(42);
        lrulist.preserve(a.clone());
        assert_eq!(Arc::strong_count(&a), 2);
        lrulist.expire(10);
        assert_eq!(Arc::strong_count(&a), 1);
    }

    #[test]
    fn expire_until() {
        crate::test::init_env_logging();

        let lrulist = LruList::<i32>::new();
        let a = Arc::new(42);
        lrulist.preserve(a.clone());
        let b = Arc::new(420);
        lrulist.preserve(b.clone());

        lrulist.expire_until(1, Box::new(move || Arc::strong_count(&b) == 0));
        assert_eq!(Arc::strong_count(&a), 1);
    }
}
