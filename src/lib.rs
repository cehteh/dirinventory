#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]
#![feature(hash_set_entry)]
#![feature(dir_entry_ext2)]
#![feature(io_error_more)]

mod gatherer;
pub use gatherer::Gatherer;

mod messages;
pub use messages::{DirectoryGatherMessage, InventoryEntryMessage};

mod objectpath;
pub use objectpath::ObjectPath;

mod internednames;
pub use internednames::{InternedName, InternedNames};

mod priority_queue;
pub use priority_queue::{PriorityQueue, QueueEntry, ReceiveGuard};
pub use openat_ct as openat;
pub use easy_error::{Error, ResultExt};

#[cfg(test)]
mod test {
    use std::{thread, time};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::io::Write;
    use std::sync::Once;

    use env_logger;

    pub fn init_env_logging() {
        static LOGGER: Once = Once::new();
        LOGGER.call_once(|| {
            let counter: AtomicU64 = AtomicU64::new(0);
            let seq_num = move || counter.fetch_add(1, Ordering::Relaxed);

            let start = time::Instant::now();

            env_logger::Builder::from_default_env()
                .format(move |buf, record| {
                    let micros = start.elapsed().as_micros() as u64;
                    writeln!(
                        buf,
                        "{:0>12}: {:0>8}.{:0>6}: {:>5}: {}:{}: {}: {}",
                        seq_num(),
                        micros / 1000000,
                        micros % 1000000,
                        record.level().as_str(),
                        record.file().unwrap_or(""),
                        record.line().unwrap_or(0),
                        thread::current().name().unwrap_or("UNKNOWN"),
                        record.args()
                    )
                })
                .try_init()
                .unwrap();
        });
    }
}
