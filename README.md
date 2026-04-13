<!--lint disable no-literal-urls-->
<div align="center">
  <h1>AV1 Obu Parser</h1>
</div>
<br/>
<div align="center">
  <strong>A pure Rust parser for AV1 OBU bitstreams and IVF containers.</strong>
</div>
<div align="center">
  <img src="https://img.shields.io/github/license/mycrl/av1-obu-parser"/>
  <img src="https://img.shields.io/github/issues/mycrl/av1-obu-parser"/>
  <img src="https://img.shields.io/github/stars/mycrl/av1-obu-parser"/>
</div>

---

`av1-obu-parser` is a bitstream analysis tool for AV1. It parses AV1 Open
Bitstream Units (OBUs), keeps the cross-frame context required by frame
headers, and can also unpack IVF containers before feeding their payloads to
the OBU parser.

This is **not** a video decoder. It does not reconstruct pixels. The goal of
this project is to inspect AV1 syntax structures and print useful stream
metadata for debugging, learning, and tooling.

## Features

- Parse AV1 OBU headers and payloads in pure Rust
- Keep parser context across Sequence Header, Frame Header, and Frame OBUs
- Read IVF file headers and iterate frame payloads with `IvfReader`
- Ship with a runnable example for inspecting `DEMO.ivf`
- Include focused unit and integration tests

## Quick Start

Clone the repository and run the test suite:

```bash
cargo test
```

The repository includes a sample IVF file at the project root:

```text
DEMO.ivf
```

## Example Program

Run the bundled example against `DEMO.ivf`:

```bash
cargo run --example simple
```

Limit the output to the first 10 OBUs:

```bash
cargo run --example simple -- --oubs 10
```

Show `TemporalDelimiter` OBUs as well so the numbering matches other analyzers:

```bash
cargo run --example simple -- --oubs 10 --show-delimiter
```

Use a different input file:

```bash
cargo run --example simple -- --input path/to/sample.ivf --oubs 20 --show-delimiter
```

## Library Usage

### Parse a raw OBU payload

```rust
use av1_obu_parser::{buffer::Buffer, obu::ObuParser};

let data: Vec<u8> = vec![];
let mut parser = ObuParser::default();
let mut buffer = Buffer::from_slice(&data);

while buffer.bytes_remaining() > 0 {
    match parser.parse(&mut buffer) {
        Ok(obu) => println!("{obu:?}"),
        Err(err) => {
            eprintln!("parse error: {err}");
            break;
        }
    }
}
```

### Parse an IVF container first

```rust
use av1_obu_parser::{buffer::Buffer, IvfReader, obu::ObuParser};

let file_data = std::fs::read("DEMO.ivf")?;
let ivf = IvfReader::new(&file_data)?;

println!(
    "codec={}, resolution={}x{}",
    ivf.header().codec_string(),
    ivf.header().width,
    ivf.header().height
);

let mut parser = ObuParser::default();
for frame in ivf.frames() {
    let frame = frame?;
    let mut buffer = Buffer::from_slice(frame.data);

    while buffer.bytes_remaining() > 0 {
        match parser.parse(&mut buffer) {
            Ok(obu) => println!("frame {}: {obu:?}", frame.index),
            Err(err) => {
                eprintln!("frame {}: {err}", frame.index);
                break;
            }
        }
    }
}

Ok::<(), Box<dyn std::error::Error>>(())
```

## Notes

- The example output is intended for debugging and comparison with other AV1
  analyzers.
- `--show-delimiter` is useful when you want local output to line up with tools
  that display every OBU, including temporal delimiters.

## License

[`MIT`](LICENSE) Copyright (c) 2026 Mycrl.
