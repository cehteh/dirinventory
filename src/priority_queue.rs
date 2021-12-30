//! A channel/message-queue based on a pairing heap with some special features:
//!
//!  * Tracks processing of messages with a ReceiveGuards and a counter. This is
//!    useful when the recevier itself may send new entrys to the queue. As long as any
//!    receiver is processing entrys, others 'recv()' are blocking. Once the queue becomes
//!    empty and the final entry is processed one waiter will get a notification with a
//!    'Drained' message.
use std::collections::BinaryHeap;
use std::sync::atomic::{self, AtomicBool, AtomicUsize};

use parking_lot::{Condvar, Mutex};
#[allow(unused_imports)]
pub use log::{debug, error, info, trace, warn};

/// A queue which orders entrys by priority (smallest first)
#[derive(Debug)]
pub struct PriorityQueue<K, P>
where
    K: Send,
    P: PartialOrd + Default + Ord,
{
    heap:        Mutex<BinaryHeap<QueueEntry<K, P>>>,
    in_progress: AtomicUsize,
    is_drained:  AtomicBool,
    notify:      Condvar,
}

impl<K, P> Default for PriorityQueue<K, P>
where
    K: Send,
    P: PartialOrd + Default + Ord,
{
    fn default() -> Self {
        Self::new()
    }
}

enum Notify {
    None,
    One,
    All,
}

impl<K, P> PriorityQueue<K, P>
where
    K: Send,
    P: PartialOrd + Default + Ord,
{
    /// Create a new PriorityQueue
    pub fn new() -> PriorityQueue<K, P> {
        PriorityQueue {
            heap:        Mutex::new(BinaryHeap::new()),
            in_progress: AtomicUsize::new(0),
            is_drained:  AtomicBool::new(true),
            notify:      Condvar::new(),
        }
    }

    /// Inserts all elements from the stash to the PriorityQueue, emties stash.
    pub fn sync(&self, stash: &mut Stash<K, P>) {
        let mut notify = Notify::None;

        if !stash.0.is_empty() {
            if stash.0.len() == 1 {
                notify = Notify::One;
            } else {
                notify = Notify::All;
            }

            let mut lock = self.heap.lock();
            stash.0.drain(..).for_each(|e| {
                lock.push(e);
            });
        }

        match notify {
            Notify::None => {}
            Notify::One => {
                self.notify.notify_one();
            }
            Notify::All => {
                self.notify.notify_all();
            }
        }
    }

    fn send_msg(&self, entry: QueueEntry<K, P>, stash: &mut Stash<K, P>) {
        let mut notify = Notify::None;

        if let Some(mut lock) = self.heap.try_lock() {
            if stash.0.is_empty() {
                notify = Notify::One;
            } else {
                notify = Notify::All;
                stash.0.drain(..).for_each(|e| {
                    lock.push(e);
                });
            }
            lock.push(entry);
        } else {
            stash.0.push(entry);
        }

        match notify {
            Notify::None => {}
            Notify::One => {
                self.notify.notify_one();
            }
            Notify::All => {
                self.notify.notify_all();
            }
        }
    }

    /// Pushes an entry with some prio onto the queue.
    pub fn send(&self, entry: K, prio: P, stash: &mut Stash<K, P>) {
        self.in_progress.fetch_add(1, atomic::Ordering::SeqCst);
        self.is_drained.store(false, atomic::Ordering::SeqCst);
        self.send_msg(QueueEntry::Entry(entry, prio), stash);
    }

    /// Send the 'Drained' message
    fn send_drained(&self) {
        if self
            .is_drained
            .compare_exchange(
                false,
                true,
                atomic::Ordering::SeqCst,
                atomic::Ordering::SeqCst,
            )
            .is_ok()
        {
            self.in_progress.fetch_add(1, atomic::Ordering::SeqCst);
            self.heap.lock().push(QueueEntry::Drained);
            self.notify.notify_one();
        }
    }

    /// Returns the smallest entry from a queue. This entry is wraped in a ReceiveGuard/QueueEntry
    pub fn recv(&self) -> ReceiveGuard<K, P> {
        let mut lock = self.heap.lock();
        while lock.is_empty() {
            self.notify.wait(&mut lock);
        }

        let entry = lock.pop().unwrap();

        ReceiveGuard::new(entry, self)
    }

    /// Try to get the smallest entry from a queue. Will return Some<ReceiveGuard> when an entry is available.
    pub fn try_recv(&self) -> Option<ReceiveGuard<K, P>> {
        match self.heap.try_lock() {
            Some(mut heap) => heap.pop().map(|entry| ReceiveGuard::new(entry, self)),
            None => None,
        }
    }
}

/// Type for the received message
#[derive(Debug, Clone, Copy)]
pub enum QueueEntry<K: Send, P: Ord> {
    /// Entry with data K and priority P
    Entry(K, P),
    /// Queue got empty and no other workers processing a ReceiveGuard
    Drained,
    /// Default value when taken from a ReceiveGuard
    Taken,
}

impl<K: Send, P: Ord> QueueEntry<K, P> {
    /// Returns a reference to the value of the entry.
    pub fn entry(&self) -> Option<&K> {
        match &self {
            QueueEntry::Entry(k, _) => Some(k),
            _ => None,
        }
    }

    /// Returns a reference to the priority of the entry.
    pub fn priority(&self) -> Option<&P> {
        match &self {
            QueueEntry::Entry(_, prio) => Some(prio),
            _ => None,
        }
    }

    /// Returns 'true' when the queue is drained
    pub fn is_drained(&self) -> bool {
        matches!(self, QueueEntry::Drained)
    }
}

impl<K: Send, P: Ord> Ord for QueueEntry<K, P> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            (QueueEntry::Entry(_, a), QueueEntry::Entry(_, b)) => b.cmp(a),
            (QueueEntry::Drained, QueueEntry::Drained) => Ordering::Equal,
            (QueueEntry::Drained, _) => Ordering::Greater,
            (_, QueueEntry::Drained) => Ordering::Less,
            (_, _) => unreachable!("'Taken' should never appear here"),
        }
    }
}

impl<K: Send, P: Ord> PartialOrd for QueueEntry<K, P> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: Send, P: Ord> PartialEq for QueueEntry<K, P> {
    fn eq(&self, other: &Self) -> bool {
        use QueueEntry::*;
        match (self, other) {
            (Entry(_, a), Entry(_, b)) => a == b,
            (Drained, Drained) | (Taken, Taken) => true,
            (_, _) => false,
        }
    }
}

impl<K: Send, P: Ord> Eq for QueueEntry<K, P> {}

impl<K: Send, P: Ord> Default for QueueEntry<K, P> {
    fn default() -> Self {
        QueueEntry::Taken
    }
}

/// Wraps a QueueEntry, when dropped and the queue is empty it sends a final 'Drained' message
/// to notify that there is no further work in progress.
#[derive(Debug)]
pub struct ReceiveGuard<'a, K, P>
where
    K: Send,
    P: PartialOrd + Default + Ord,
{
    entry: QueueEntry<K, P>,
    pq:    &'a PriorityQueue<K, P>,
}

impl<'a, K, P> ReceiveGuard<'a, K, P>
where
    K: Send,
    P: PartialOrd + Default + Ord,
{
    fn new(entry: QueueEntry<K, P>, pq: &'a PriorityQueue<K, P>) -> Self {
        ReceiveGuard { entry, pq }
    }

    /// Takes the 'QueueEntry' entry out of a ReceiveGuard, drop the guard (and may by that send the 'Drained' message).
    pub fn entry(&self) -> &QueueEntry<K, P> {
        &self.entry
    }

    /// Takes the 'QueueEntry' entry out of a ReceiveGuard, drop the guard (and may by that send the 'Drained' message).
    pub fn into_entry(mut self) -> QueueEntry<K, P> {
        std::mem::take(&mut self.entry)
    }
}

impl<K, P> Drop for ReceiveGuard<'_, K, P>
where
    K: Send,
    P: PartialOrd + Default + Ord,
{
    fn drop(&mut self) {
        if self.pq.in_progress.fetch_sub(1, atomic::Ordering::SeqCst) == 1 {
            self.pq.send_drained()
        }
    }
}

/// For (mostly) contentions free queue insertion every thread maintains a private
/// 'stash'. When messages are send to the PriorityQueue while it is locked they are stored in
/// this stash. Once the lock is obtained the stashed messages will be moved to the
/// PriorityQueue. This can also be enforced with the 'PriorityQueue::sync()' function.
pub struct Stash<K, P>(Vec<QueueEntry<K, P>>)
where
    K: Send,
    P: PartialOrd + Default + Ord;

impl<K, P> Stash<K, P>
where
    K: Send,
    P: PartialOrd + Default + Ord,
{
    /// Creates a new stash.
    pub fn new() -> Self {
        Stash(Vec::new())
    }
}

impl<K, P> Default for Stash<K, P>
where
    K: Send,
    P: PartialOrd + Default + Ord,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::sync::Arc;

    use super::{PriorityQueue, QueueEntry, Stash};
    use crate::test;

    #[test]
    fn smoke() {
        test::init_env_logging();
        let queue: PriorityQueue<String, u64> = PriorityQueue::new();
        let mut stash = Stash::<String, u64>::new();
        queue.send("test 1".to_string(), 1, &mut stash);
        queue.send("test 3".to_string(), 3, &mut stash);
        queue.send("test 2".to_string(), 2, &mut stash);
        assert_eq!(
            queue.recv().entry(),
            &QueueEntry::Entry("test 1".to_string(), 1)
        );
        assert_eq!(
            queue.recv().entry(),
            &QueueEntry::Entry("test 2".to_string(), 2)
        );
        assert_eq!(
            queue.recv().entry(),
            &QueueEntry::Entry("test 3".to_string(), 3)
        );
        assert_eq!(queue.recv().entry(), &QueueEntry::Drained);
        assert!(queue.try_recv().is_none());
    }

    #[test]
    fn try_recv() {
        test::init_env_logging();
        let queue: PriorityQueue<String, u64> = PriorityQueue::new();
        let mut stash = Stash::<String, u64>::new();
        queue.send("test 1".to_string(), 1, &mut stash);
        queue.send("test 3".to_string(), 3, &mut stash);
        queue.send("test 2".to_string(), 2, &mut stash);
        assert!(queue.try_recv().is_some());
        assert!(queue.try_recv().is_some());
        assert!(queue.try_recv().is_some());
        assert!(queue.try_recv().is_some());
        assert!(queue.try_recv().is_none());
        assert!(queue.try_recv().is_none());
    }

    #[test]
    fn threads() {
        test::init_env_logging();
        let queue: Arc<PriorityQueue<String, u64>> = Arc::new(PriorityQueue::new());

        let thread1_queue = queue.clone();
        let mut stash1 = Stash::<String, u64>::new();
        let thread1 = thread::spawn(move || {
            thread1_queue.send("test 1".to_string(), 1, &mut stash1);
            thread1_queue.send("test 3".to_string(), 3, &mut stash1);
            thread1_queue.send("test 2".to_string(), 2, &mut stash1);
        });
        thread1.join().unwrap();

        let thread2_queue = queue.clone();
        let mut stash2 = Stash::<String, u64>::new();
        let thread2 = thread::spawn(move || {
            assert_eq!(
                thread2_queue.recv().entry(),
                &QueueEntry::Entry("test 1".to_string(), 1)
            );
            assert_eq!(
                thread2_queue.recv().entry(),
                &QueueEntry::Entry("test 2".to_string(), 2)
            );
            assert_eq!(
                thread2_queue.recv().entry(),
                &QueueEntry::Entry("test 3".to_string(), 3)
            );
            assert!(thread2_queue.recv().entry().is_drained());
            assert!(thread2_queue.try_recv().is_none());
            thread2_queue.send("test 4".to_string(), 4, &mut stash2);
            assert_eq!(
                thread2_queue.recv().entry(),
                &QueueEntry::Entry("test 4".to_string(), 4)
            );
            assert!(thread2_queue.recv().entry().is_drained());
            assert!(thread2_queue.try_recv().is_none());
        });

        thread2.join().unwrap();
    }
}
