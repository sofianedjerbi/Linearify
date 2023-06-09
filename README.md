<p align="center">
  <img src="logo.png" width="400" style="max-width: 100%; height: auto;" alt="Logo">
</p>

# Quickstart

Here is an example using [FastNBT](https://github.com/owengage/fastnbt) to print a whole chunk.

```rust
use fastnbt::Value;
use linearify::{self, Region};

fn main() {
    let mut region = linearify::open_linear("./r.0.0.linear").unwrap();
    let buf = region.chunks[0].clone().unwrap().raw_chunk;
    let val: Value = fastnbt::from_bytes(buf.as_slice()).unwrap();
    println!("{:?}", val);
}
```