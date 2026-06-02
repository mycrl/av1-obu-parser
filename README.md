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
- Include focused unit and integration tests

## Example Program

Run the bundled example against `sample.ivf`:

```bash
cargo run --example simple
```

Limit the output to the first 10 OBUs:

```bash
cargo run --example simple -- --obus 10
```

Show `TemporalDelimiter` OBUs as well so the numbering matches other analyzers:

```bash
cargo run --example simple -- --obus 10 --show-delimiter
```

Use a different input file:

```bash
cargo run --example simple -- --input path/to/sample.ivf --obus 20 --show-delimiter
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

A typical sequence header output

```rust
SequenceHeader(
    SequenceHeader {
        seq_profile: Main,
        still_picture: false,
        reduced_still_picture_header: false,
        timing_info: None,
        decoder_model_info: None,
        initial_display_delay_present_flag: false,
        operating_points: [
            OperatingPoint {
                idc: 0,
                level_idx: 31,
                tier: false,
                operating_parameters_info: None,
                initial_display_delay: 10,
            },
        ],
        frame_width_bits: 9,
        frame_height_bits: 8,
        max_frame_width: 320,
        max_frame_height: 180,
        frame_id_numbers_present: None,
        use_128x128_superblock: false,
        enable_filter_intra: false,
        enable_intra_edge_filter: false,
        enable_interintra_compound: false,
        enable_masked_compound: false,
        enable_warped_motion: false,
        enable_dual_filter: false,
        enable_order_hint: true,
        enable_jnt_comp: false,
        enable_ref_frame_mvs: false,
        seq_choose_screen_content_tools: false,
        seq_force_screen_content_tools: 0,
        seq_force_integer_mv: 2,
        enable_superres: false,
        enable_cdef: true,
        enable_restoration: true,
        color_config: ColorConfig {
            high_bitdepth: false,
            twelve_bit: false,
            mono_chrome: false,
            color_description_present: false,
            color_primaries: Unspecified,
            transfer_characteristics: Unspecified,
            matrix_coefficients: Unspecified,
            color_range: false,
            subsampling_x: true,
            subsampling_y: true,
            chroma_sample_position: Some(
                Unknown,
            ),
            separate_uv_delta_q: false,
        },
        film_grain_params_present: false,
    },
)
```

## License

[`MIT`](LICENSE) Copyright (c) 2026 Mycrl.
