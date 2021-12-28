# Description

This library implements machinery for extremely fast multithreaded directory traversal.
Using multiple threads leverages the kernels abilities to schedule IO-Requests in an
optimal way.

# How it works

The user crates a 'Gatherer' object which spawns threads listening on an
PriorityQueue. Sending a 'directory' to this queue let one thread pick it up and traverse the
directory. Each element found is then send to a custom function/closure which may decides on
how to process it:
 * Directories can be send again into the input PriorityQueue where other
   threads may pick them up. This happens until the input queue is exhausted, eventually traversing
   all sub-directories of the directory send initially.
 * Files and Directories can be send to an output mcmp queue where they can be further
   processed.
 * Errors are send to the output queue as they happen.
 * Once the input queue becomes empty a 'Done' message is send to the output to notify the
   listener there.

## Queues

A priority queue is choosen for the input to ensure that directories are processed in a file
handled preserving order. This is depth first in ascending inode order.

## Memory Optimizations

Handling pathnames of millions of files would need considerably much memory. To conserve this
demands a ObjectPath implementation encodes any path by its filename and a reference to its
parent directory. Futher all names are interned thus same names would require only memory once
for their storage.

# Benchmarking Results

See the 'BENCH.md' file for some tests. As baseline was the 'gnu find' utility chosen. In the
most extreme case this code can perform directory traversal 20 times faster. With slow
spinning disks and moderate settings (16 threads) 1.6 times faster.
