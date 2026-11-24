### Changed

- Building the drawn surface fills its buffers and finds its edges on all threads: at 4 M cells the default surface takes 0.3 s instead of 0.6 s, a colour change by cell type 0.34 s instead of 0.53 s, and the exact cut after a clip drag 0.93 s instead of 1.28 s. The picture is the same, bit for bit. `murmel bench` reports the phases of every surface stage.
