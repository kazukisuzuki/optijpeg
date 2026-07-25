# optijpeg

Optimizes JPEG files losslessly in place.

It preserves the quantized DCT coefficients and produces an optimized progressive JPEG.

All APP markers and comments are removed, including EXIF and ICC profiles.


## Build

~~~sh
cargo build --release
~~~


## Install

~~~sh
cargo install --force --git https://github.com/kazukisuzuki/optijpeg.git
~~~


## Uninstall

~~~sh
cargo uninstall optijpeg
~~~


## Usage

~~~
optijpeg image1.jpg image2.jpg
optijpeg -r ./images1 ./images2
~~~

`-r/--recursive` accepts one or more directories and processes files with
`.jpg` or `.jpeg` extensions, case-insensitively. It cannot be combined with
direct file arguments. Symbolic links are ignored. Direct file arguments may
use any extension. Files are processed in parallel, and each result is printed as
soon as it finishes.

A file without APP or comment markers is left unchanged unless optimization
makes it smaller. A file containing those markers is replaced even when
removing them makes the result larger. Processing continues after errors, and
the exit status is non-zero if any file or recursive path fails.
