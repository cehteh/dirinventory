# Benchmark Notes

Benchmarking filesystem peformance is hard and not very well covered in rusts bench tools. For
the time being I add some commented test runs here.

There are 1 kinds of benchmarks I can run here:

 1. Big backup directory on Raid6 HDD's with a linux dmcache on top.  This contains roughly
    5TB/200Million massively hardlinked entries. As it is a production system the amount of
    files may vary slightly. The dmcache is 'hot' because of the multiple test runs I already done.

 2. External (laptop) HDD on USB3.  Some dirs/files (655021 objects) from the above backupset are copied onto
    this disk. This allows testing a slower spinning disk.

Between each test the kernel caches are cleared by 'echo 3 >/proc/sys/vm/drop_cache'

For benchmarking the 'gatherer::test::load_dir' in release build mode is used. Rebuild with
different num_gatherer_threads and inventory_backlog. 'RUST_LOG' is set to 'off'.
The gnu 'find' tool is used to give a baseline.

Tests are run with '/usr/bin/time -v' to get the time and resource usage.

Testing machine is a 'AMD Ryzen 9 3900X 12-Core Processor' with 64GB RAM, not completely idle.

Filesystem is ext4.

## Initial Observations

 * Terminals are slow!  While running slow tests the output/write to a terminal is not a
   limiting factor it eventually becomes so when listing directories from fast SSD's or
   caches.  The tests are run with the output piped to 'wc -l'. Since the
   'gatherer::test::load_dir' test uses a single thread for output this is still somewhat
   limiting. In a real application it may be feasible to consume the output by multiple
   threads.

 * To my own surprise this scales with a lot of threads.

 * With very many threads (threads * deepest_nested_dir > 1000) and deeply nested directories 'dirinventory' may run out of file
   handles. This is a known defect and will be fixed in future versions.

 * Timing between multiple runs of these benchmarks is reasonably consistent, thus only one
   run is shown here.

# The benchmarks

Command used are:

```
/usr/bin/time -v find | wc -l
```

```
/usr/bin/time -v /home/ct/src/dirinventory/target/release/deps/dirinventory-5f174d3bd0bc0738 load_dir --ignored --nocapture | wc -l
```

## The big backup directory (from 1.)

0. find
```
	Command being timed: "find"
	User time (seconds): 102.16
	System time (seconds): 514.58
	Percent of CPU this job got: 18%
	Elapsed (wall clock) time (h:mm:ss or m:ss): 54:20.20
	Average shared text size (kbytes): 0
	Average unshared data size (kbytes): 0
	Average stack size (kbytes): 0
	Average total size (kbytes): 0
	Maximum resident set size (kbytes): 6792
	Average resident set size (kbytes): 0
	Major (requiring I/O) page faults: 11
	Minor (reclaiming a frame) page faults: 44466
	Voluntary context switches: 20507834
	Involuntary context switches: 29240
	Swaps: 0
	File system inputs: 187033032
	File system outputs: 0
	Socket messages sent: 0
	Socket messages received: 0
	Signals delivered: 0
	Page size (bytes): 4096
	Exit status: 0
212493384

```


1. gatherer::test::load_dir 8 threads / 65536 backlog
```

```


2. gatherer::test::load_dir 16 threads / 65536 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 572.92
System time (seconds): 1184.87
Percent of CPU this job got: 391%
Elapsed (wall clock) time (h:mm:ss or m:ss): 7:29.27
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 92976
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 6
Minor (reclaiming a frame) page faults: 21942
Voluntary context switches: 70630677
Involuntary context switches: 632471
Swaps: 0
File system inputs: 179679824
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
212493390
```

3. gatherer::test::load_dir 32 threads / 65536 backlog
```

```

4. gatherer::test::load_dir 32 threads / 524288 backlog
```

```

5. gatherer::test::load_dir 64 threads / 524288 backlog
```

```

6. gatherer::test::load_dir 128 threads / 524288 backlog
```

```

## The external hard drive (from 2.)

0. find
```

```

1. gatherer::test::load_dir 8 threads / 65536 backlog
```

```

2. gatherer::test::load_dir 16 threads / 65536 backlog
```

```

3. gatherer::test::load_dir 32 threads / 65536 backlog
```

```

4. gatherer::test::load_dir 32 threads / 524288 backlog
```

```

5. gatherer::test::load_dir 64 threads / 524288 backlog
```

```

6. gatherer::test::load_dir 128 threads / 524288 backlog
```

```
