# Benchmark Notes

Benchmarking filesystem peformance is hard and not very well covered in rusts bench tools. For
the time being I add some commented test runs here.

There are 1 kinds of benchmarks I run here:

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
   caches. Thus most tests are run with the output piped to 'wc -l'.
   'gatherer::test::load_dir' test uses a single thread for output this is still somewhat
   limiting, for the tests where this limits are significant the output is disabled, this is
   noted in the test description. In a real application it is recommended to consume the
   output by multiple threads.

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

1. gatherer::test::load_dir 1 threads / 65536 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 587.00
System time (seconds): 633.54
Percent of CPU this job got: 74%
Elapsed (wall clock) time (h:mm:ss or m:ss): 27:22.23
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 78456
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 6
Minor (reclaiming a frame) page faults: 18319
Voluntary context switches: 40162725
Involuntary context switches: 70781
Swaps: 0
File system inputs: 181338904
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
212910207
```

2. gatherer::test::load_dir 8 threads / 65536 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 542.97
System time (seconds): 892.56
Percent of CPU this job got: 191%
Elapsed (wall clock) time (h:mm:ss or m:ss): 12:30.84
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 88884
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 6
Minor (reclaiming a frame) page faults: 21460
Voluntary context switches: 44527743
Involuntary context switches: 106561
Swaps: 0
File system inputs: 180859416
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
212910207
```

3. gatherer::test::load_dir 16 threads / 65536 backlog
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

4. gatherer::test::load_dir 32 threads / 65536 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 542.97
System time (seconds): 892.56
Percent of CPU this job got: 191%
Elapsed (wall clock) time (h:mm:ss or m:ss): 12:30.84
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 88884
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 6
Minor (reclaiming a frame) page faults: 21460
Voluntary context switches: 44527743
Involuntary context switches: 106561
Swaps: 0
File system inputs: 180859416
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
212910207
```

5. gatherer::test::load_dir 32 threads / 524288 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 2245.90
System time (seconds): 1969.46
Percent of CPU this job got: 796%
Elapsed (wall clock) time (h:mm:ss or m:ss): 8:49.23
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 216396
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 9
Minor (reclaiming a frame) page faults: 44653
Voluntary context switches: 189613341
Involuntary context switches: 51248161
Swaps: 0
File system inputs: 180784472
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
212493390
```

6. gatherer::test::load_dir 32 threads / 524288 backlog / Output disabled
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 687.09
System time (seconds): 1737.09
Percent of CPU this job got: 919%
Elapsed (wall clock) time (h:mm:ss or m:ss): 4:23.73
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 113712
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 9
Minor (reclaiming a frame) page faults: 18976
Voluntary context switches: 108985944
Involuntary context switches: 3200705
Swaps: 0
File system inputs: 180190184
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
7
```

7. gatherer::test::load_dir 64 threads / 524288 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 685.07
System time (seconds): 2805.48
Percent of CPU this job got: 1689%
Elapsed (wall clock) time (h:mm:ss or m:ss): 3:26.64
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 119960
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 3
Minor (reclaiming a frame) page faults: 20521
Voluntary context switches: 144230796
Involuntary context switches: 21093983
Swaps: 0
File system inputs: 180819768
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
7
```

8. gatherer::test::load_dir 128 threads / 524288 backlog / Output disabled
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 667.76
System time (seconds): 3575.21
Percent of CPU this job got: 1967%
Elapsed (wall clock) time (h:mm:ss or m:ss): 3:35.64
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 166172
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 7
Minor (reclaiming a frame) page faults: 31526
Voluntary context switches: 164665662
Involuntary context switches: 24296202
Swaps: 0
File system inputs: 180463424
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
7
```

## The external hard drive (from 2.)

0. find
```
Command being timed: "find"
User time (seconds): 0.62
System time (seconds): 3.47
Percent of CPU this job got: 1%
Elapsed (wall clock) time (h:mm:ss or m:ss): 4:50.03
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 3388
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 10
Minor (reclaiming a frame) page faults: 185
Voluntary context switches: 131824
Involuntary context switches: 130986
Swaps: 0
File system inputs: 1326016
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
655021
```

1. gatherer::test::load_dir 1 threads / 65536 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 2.99
System time (seconds): 4.47
Percent of CPU this job got: 3%
Elapsed (wall clock) time (h:mm:ss or m:ss): 3:52.79
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 10584
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 5
Minor (reclaiming a frame) page faults: 1624
Voluntary context switches: 262238
Involuntary context switches: 131269
Swaps: 0
File system inputs: 1332968
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
655027
```

2. gatherer::test::load_dir 8 threads / 65536 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 3.01
System time (seconds): 5.39
Percent of CPU this job got: 4%
Elapsed (wall clock) time (h:mm:ss or m:ss): 3:09.14
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
verage total size (kbytes): 0
Maximum resident set size (kbytes): 10556
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 151
Minor (reclaiming a frame) page faults: 1717
Voluntary context switches: 249760
Involuntary context switches: 118901
Swaps: 0
File system inputs: 1325336
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
655027
```

3. gatherer::test::load_dir 16 threads / 65536 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 2.97
System time (seconds): 5.74
Percent of CPU this job got: 4%
Elapsed (wall clock) time (h:mm:ss or m:ss): 2:58.44
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 13588
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 4
Minor (reclaiming a frame) page faults: 1889
Voluntary context switches: 254570
Involuntary context switches: 118243
Swaps: 0
File system inputs: 1334512
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
655027
```

4. gatherer::test::load_dir 32 threads / 65536 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 3.17
System time (seconds): 5.63
Percent of CPU this job got: 5%
Elapsed (wall clock) time (h:mm:ss or m:ss): 2:50.29
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 14672
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 4
Minor (reclaiming a frame) page faults: 2126
Voluntary context switches: 256248
Involuntary context switches: 114466
Swaps: 0
File system inputs: 1334520
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
655027
```

5. gatherer::test::load_dir 32 threads / 524288 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 3.16
System time (seconds): 5.71
Percent of CPU this job got: 5%
Elapsed (wall clock) time (h:mm:ss or m:ss): 2:55.53
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 43152
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 4
Minor (reclaiming a frame) page faults: 2112
Voluntary context switches: 255572
Involuntary context switches: 114915
Swaps: 0
File system inputs: 1335520
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
655027
```

6. gatherer::test::load_dir 64 threads / 524288 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 3.01
System time (seconds): 5.96
Percent of CPU this job got: 5%
Elapsed (wall clock) time (h:mm:ss or m:ss): 2:41.82
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 46612
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 153
Minor (reclaiming a frame) page faults: 2010
Voluntary context switches: 248231
Involuntary context switches: 74601
Swaps: 0
File system inputs: 1324312
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
655027
```

7. gatherer::test::load_dir 128 threads / 524288 backlog
```
Command being timed: "/home/ct/src/dirinventory/target/release/deps/dirinventory-757eb313ebed9060 load_dir --ignored --nocapture"
User time (seconds): 3.07
System time (seconds): 7.32
Percent of CPU this job got: 6%
Elapsed (wall clock) time (h:mm:ss or m:ss): 2:51.73
Average shared text size (kbytes): 0
Average unshared data size (kbytes): 0
Average stack size (kbytes): 0
Average total size (kbytes): 0
Maximum resident set size (kbytes): 55892
Average resident set size (kbytes): 0
Major (requiring I/O) page faults: 156
Minor (reclaiming a frame) page faults: 3323
Voluntary context switches: 300231
Involuntary context switches: 60541
Swaps: 0
File system inputs: 1325328
File system outputs: 0
Socket messages sent: 0
Socket messages received: 0
Signals delivered: 0
Page size (bytes): 4096
Exit status: 0
655027
```

# Conclusions

* This is about reordering I/O requests, the number of gathering threads should not be tied to
  the number of CPU cores available.

* In the one-thead case there are already improvemnts because there is the single gathering
  thread which is decoupled from the output, running in the main thread.

* It does not scale linear, eventually thread scheduling overhead makes it worse.

## Data from SSD Cache

* SSD's scale extremely well in the most extreme case it 20 times faster than the 'find'
  baseline test. Other factors like how fast one can process the output become predominant
  then. Even with only 16 Threads it is more then 4 times faster than the baseline. With 32
  threads, bigger backlog and output disabled it becomes more than 12 times as fast.

* Not surprisingly, more threads need longer backlog as well.

## Data from slow HDD

* Moving mechanical heads is the most limiting factor here. Still in the best case it is 1.8
  times faster than the baseline.

* Backlog size is not an issue here since the disk is so slow that data always be processed in
  time (not clearing the page cache in RAM may change this, but this is deliberately not
  tested).

* It still scales with more threads, but the benefits are not as much.
