# bzip2 test corpus

These are selected small cases from maintained upstream test suites. They are
kept in the repository so correctness tests are deterministic and need no
network access.

- `go/` and `lbzip2/` come from the official
  [`bzip2-testfiles`](https://gitlab.com/bzip2/bzip2-testfiles) collection.
  Files ending in `.bz2` are valid and files ending in `.bz2.bad` are
  deliberately corrupt. Each directory contains its upstream license.

Legacy randomized blocks produced by bzip2 versions before 0.9.5 are excluded:
supporting this obsolete format would complicate the production decoder and
its hot path for no realistic modern input.

The uncompressed reference bytes are deliberately not stored. Tests decode
valid inputs with the maintained pure-Rust `libbz2-rs-sys` implementation and
compare `fbz` byte-for-byte with that differential oracle.
