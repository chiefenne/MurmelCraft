### Changed

- The window shows a mesh as soon as it is read and checks the SHA-256 of its files afterwards; the validity badge says Checking until the digest is in place. Without SHA instructions (as on a 2019 Core i9) a 4 M cell ascii `.vtu` shows after 9.4 s instead of 16.6 s. `murmel bench` times the wait for the digest (`open.digest`).
