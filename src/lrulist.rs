use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver, Sender};

pub(crate) struct LruList<T>(Sender<Arc<T>>, Receiver<Arc<T>>);

impl<T> LruList<T> {
    pub fn new() -> LruList<T> {
        let (s, r) = unbounded();
        LruList(s, r)
    }

    pub fn preserve(&self, element: Arc<T>) {
        let _ = self.0.send(element);
    }

    pub fn expire(&self, mut n: usize) -> bool {
        while n >= 1 {
            match self.1.try_recv() {
                Ok(elem) if Arc::strong_count(&elem) == 1 => {
                    n -= 1;
                }
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        true
    }

    pub fn expire_all(&self) {
        self.expire(usize::MAX);
    }

    pub fn expire_until(&self, batch: usize, predicate: Box<dyn Fn() -> bool>) {
        while !predicate() {
            if !self.expire(batch) {
                return;
            }
        }
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
