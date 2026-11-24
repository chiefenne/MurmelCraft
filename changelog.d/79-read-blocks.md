### Added

- The VTKHDF reader can read the cells of some blocks of a blocked file (`vtkhdf::read_blocks`): each block's own rows, with the halo of ghost cells around it when asked, the fields at those cells and the boundary faces they own. One block of 65,536 cells of a 4 M cell file reads in about 0.2 s. This is the base for regions of a mesh beyond the memory budget. A block the file does not have is BLOCK005.
