# Third-party software shipped in this package

## rclone

The `libexec/bufonidae/rclone` binary is rclone, unmodified, from
https://rclone.org (the release named in `docs/COMPATIBILITY.md`).

    Copyright (C) 2012 by Nick Craig-Wood https://www.craig-wood.com/nick/

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in
    all copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
    THE SOFTWARE.

rclone is a separate program with its own security updates. Bufonidae uses this
private copy and never replaces the rclone of your system.

## Rust libraries

Bufonidae is built from Rust crates under their own licenses (mostly MIT and
Apache-2.0); the full list is checked by `cargo deny` on every release.
