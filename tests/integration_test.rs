/// AV1 OBU parser integration tests.
///
/// Contents:
/// 1. Buffer unit tests — bit-reading primitive verification.
/// 2. Sequence header parsing test — hand-crafted bitstream.
/// 3. Full bitstream parsing test — optional IVF fixture file.
///
/// Hand-crafted test vectors note:
/// AV1 bitstreams are MSB-first: within each byte the most significant bit
/// is read first.
use av1_obu_parser::{
    IvfReader,
    buffer::Buffer,
    obu::{Obu, ObuParser},
};

// ─────────────────────────────────────────────────────────────
// Test helper: BitWriter
// ─────────────────────────────────────────────────────────────

/// Bitstream writer for constructing precise AV1 test vectors.
///
/// AV1 uses MSB-first bit ordering: within each byte the most significant
/// bit is written first.
struct BitWriter {
    bytes: Vec<u8>,
    current_byte: u8,
    bit_pos: usize, // bits written into the current byte (0–7)
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            current_byte: 0,
            bit_pos: 0,
        }
    }

    /// Write a single bit.
    fn write_bit(&mut self, bit: bool) {
        if bit {
            self.current_byte |= 1 << (7 - self.bit_pos);
        }
        self.bit_pos += 1;
        if self.bit_pos == 8 {
            self.bytes.push(self.current_byte);
            self.current_byte = 0;
            self.bit_pos = 0;
        }
    }

    /// Write `count` bits MSB-first.
    fn write_bits(&mut self, value: u32, count: usize) {
        for i in (0..count).rev() {
            self.write_bit((value >> i) & 1 != 0);
        }
    }

    /// Finish writing and return the byte vector (trailing bits are zero-padded).
    fn finish(mut self) -> Vec<u8> {
        if self.bit_pos > 0 {
            self.bytes.push(self.current_byte);
        }
        self.bytes
    }

    /// Write a LEB128-encoded unsigned integer.
    fn write_leb128(&mut self, mut value: u64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                self.write_bits(byte as u32, 8);
                break;
            } else {
                self.write_bits((byte | 0x80) as u32, 8);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Test fixture: 640×480 sequence header OBU
// ─────────────────────────────────────────────────────────────

/// Build a valid AV1 Sequence Header OBU for a 640×480 Main Profile still picture.
///
/// Uses reduced_still_picture_header = 1 to keep the structure minimal.
///
/// Bit layout (AV1 spec Section 5.5 - sequence_header_obu()):
/// - seq_profile = 0 (3 bits)
/// - still_picture = 1 (1 bit)
/// - reduced_still_picture_header = 1 (1 bit)
/// - seq_level_idx[0] = 0 (5 bits, Level 2.0)
/// - frame_width_bits_minus_1 = 9 (4 bits → 10-bit width field)
/// - frame_height_bits_minus_1 = 8 (4 bits → 9-bit height field)
/// - max_frame_width_minus_1 = 639 (10 bits) → width 640
/// - max_frame_height_minus_1 = 479 (9 bits) → height 480
/// - use_128x128_superblock = 0
/// - enable_filter_intra = 1
/// - enable_intra_edge_filter = 1
/// - enable_superres = 0
/// - enable_cdef = 1
/// - enable_restoration = 1
/// - color_config (8-bit depth, YUV 4:2:0, BT.601 limited range)
/// - film_grain_params_present = 0
fn build_sequence_header_obu_640x480() -> Vec<u8> {
    let mut payload = BitWriter::new();

    // seq_profile = 0 (Main Profile)
    payload.write_bits(0, 3);
    // still_picture = 1
    payload.write_bit(true);
    // reduced_still_picture_header = 1
    payload.write_bit(true);
    // seq_level_idx[0] = 0 (Level 2.0)
    payload.write_bits(0, 5);

    // frame_width_bits_minus_1 = 9 (10 bits needed to represent 640)
    payload.write_bits(9, 4);
    // frame_height_bits_minus_1 = 8 (9 bits needed to represent 480)
    payload.write_bits(8, 4);
    // max_frame_width_minus_1 = 639 (10 bits)
    payload.write_bits(639, 10);
    // max_frame_height_minus_1 = 479 (9 bits)
    payload.write_bits(479, 9);

    // use_128x128_superblock = 0 (64×64 superblocks)
    payload.write_bit(false);
    // enable_filter_intra = 1
    payload.write_bit(true);
    // enable_intra_edge_filter = 1
    payload.write_bit(true);

    // enable_superres = 0
    payload.write_bit(false);
    // enable_cdef = 1
    payload.write_bit(true);
    // enable_restoration = 1
    payload.write_bit(true);

    // color_config fields:
    // high_bitdepth = 0 (8-bit depth)
    payload.write_bit(false);
    // mono_chrome = 0 (colour)
    payload.write_bit(false);
    // color_description_present_flag = 0 (default colour description)
    payload.write_bit(false);
    // color_range = 0 (studio/limited range)
    payload.write_bit(false);
    // Main Profile fixes subsampling_x=1 subsampling_y=1 (YUV 4:2:0)
    // chroma_sample_position = 0 (unknown/colocated)
    payload.write_bits(0, 2);
    // separate_uv_delta_q = 0
    payload.write_bit(false);

    // film_grain_params_present = 0
    payload.write_bit(false);

    let payload_bytes = payload.finish();

    // OBU header byte: forbidden=0, type=1 (SequenceHeader), extension=0,
    //                  has_size=1, reserved=0  →  0b0_0001_0_1_0 = 0x0A
    let obu_header = 0x0Au8;

    let mut obu = vec![obu_header];
    let size = payload_bytes.len() as u64;
    let mut size_writer = BitWriter::new();
    size_writer.write_leb128(size);
    obu.extend(size_writer.finish());
    obu.extend(payload_bytes);
    obu
}

// ─────────────────────────────────────────────────────────────
// Test cases
// ─────────────────────────────────────────────────────────────

/// Parse a hand-crafted 640×480 Main Profile sequence header OBU.
#[test]
fn test_parse_sequence_header_640x480() {
    let obu_bytes = build_sequence_header_obu_640x480();
    let mut parser = ObuParser::default();
    let mut buf = Buffer::from_slice(&obu_bytes);

    let result = parser
        .parse(&mut buf)
        .expect("sequence header should parse successfully");

    match result {
        Obu::SequenceHeader(seq) => {
            use av1_obu_parser::obu::sequence_header::SequenceProfile;
            assert_eq!(seq.seq_profile, SequenceProfile::Main);
            assert_eq!(seq.max_frame_width, 640);
            assert_eq!(seq.max_frame_height, 480);
            assert!(seq.still_picture);
            assert!(seq.reduced_still_picture_header);
            assert!(!seq.use_128x128_superblock);
            assert!(seq.enable_filter_intra);
            assert!(seq.enable_intra_edge_filter);
            assert!(!seq.enable_superres);
            assert!(seq.enable_cdef);
            assert!(seq.enable_restoration);
            assert!(!seq.color_config.high_bitdepth);
            assert!(!seq.color_config.mono_chrome);
            assert!(seq.color_config.subsampling_x);
            assert!(seq.color_config.subsampling_y);
            assert!(!seq.film_grain_params_present);
        }
        other => panic!("expected SequenceHeader, got {:?}", other),
    }
}

/// Parse a hand-crafted Temporal Delimiter OBU.
#[test]
fn test_parse_obu_header_types() {
    // Temporal Delimiter: type=2, extension=0, has_size=1
    // 0b0_0010_0_1_0 = 0x12
    let temporal_delimiter_obu = vec![
        0x12u8, // OBU header: type=2 (TemporalDelimiter), has_size=1
        0x00,   // obu_size = 0 (empty payload)
    ];

    let mut parser = ObuParser::default();
    let mut buf = Buffer::from_slice(&temporal_delimiter_obu);
    let result = parser
        .parse(&mut buf)
        .expect("temporal delimiter should parse successfully");

    assert!(
        matches!(result, Obu::TemporalDelimiter),
        "expected TemporalDelimiter"
    );
}

/// Basic bit-reading from Buffer.
#[test]
fn test_buffer_basic_read() {
    // 0b10110010 = 0xB2
    let data = [0xB2u8];
    let mut buf = Buffer::from_slice(&data);

    assert_eq!(buf.get_bit(), true, "bit 7 should be 1");
    assert_eq!(buf.get_bit(), false, "bit 6 should be 0");
    assert_eq!(buf.get_bit(), true, "bit 5 should be 1");
    assert_eq!(buf.get_bit(), true, "bit 4 should be 1");
    assert_eq!(buf.get_bit(), false, "bit 3 should be 0");
    assert_eq!(buf.get_bit(), false, "bit 2 should be 0");
    assert_eq!(buf.get_bit(), true, "bit 1 should be 1");
    assert_eq!(buf.get_bit(), false, "bit 0 should be 0");
}

/// get_bits across a byte boundary.
#[test]
fn test_buffer_get_bits_crossbyte() {
    // 0xAB 0xCD = 1010_1011 1100_1101
    let data = [0xABu8, 0xCDu8];
    let mut buf = Buffer::from_slice(&data);

    // 12 bits: 1010_1011_1100 = 0xABC
    assert_eq!(buf.get_bits(12), 0xABC);
    // remaining 4 bits: 1101 = 0xD
    assert_eq!(buf.get_bits(4), 0xD);
}

/// LEB128 decoding for single-byte and multi-byte values.
#[test]
fn test_buffer_leb128() {
    // Single byte: 5
    let data1 = [0x05u8];
    let mut buf1 = Buffer::from_slice(&data1);
    assert_eq!(buf1.get_leb128(), 5);

    // Two bytes: 128 → LEB128 [0x80, 0x01]
    let data2 = [0x80u8, 0x01u8];
    let mut buf2 = Buffer::from_slice(&data2);
    assert_eq!(buf2.get_leb128(), 128);

    // Two bytes: 300 → LEB128 [0xAC, 0x02]
    // 300 = 0b1_0010_1100 → low7 = 0x2C, high = 0x02
    let data3 = [0xACu8, 0x02u8];
    let mut buf3 = Buffer::from_slice(&data3);
    assert_eq!(buf3.get_leb128(), 300);
}

/// get_su(): signed integer with sign-bit extension.
#[test]
fn test_buffer_get_su() {
    // su(4): read 1100 = 12; sign bit set → 12 - 16 = -4
    let data = [0b1100_0000u8];
    let mut buf = Buffer::from_slice(&data);
    assert_eq!(buf.get_su(4), -4);

    // su(4): read 0011 = 3; sign bit clear → 3
    let data2 = [0b0011_0000u8];
    let mut buf2 = Buffer::from_slice(&data2);
    assert_eq!(buf2.get_su(4), 3);

    // su(8): read 0xFF = 255; sign bit set → 255 - 256 = -1
    let data3 = [0xFFu8];
    let mut buf3 = Buffer::from_slice(&data3);
    assert_eq!(buf3.get_su(8), -1);
}

/// get_ns(): non-symmetric unsigned integer.
#[test]
fn test_buffer_get_ns() {
    // ns(5): n=5, w=3, m=8-5=3
    // Values 0–2 use 2 bits; values 3–4 use 3 bits.
    // 00  → v=0, v<m=3 → 0
    // 01  → v=1, v<m=3 → 1
    // 10  → v=2, v<m=3 → 2
    // 110 → v=3, v>=m, extra bit 0 → (3<<1)-3+0 = 3
    // 111 → v=3, v>=m, extra bit 1 → (3<<1)-3+1 = 4
    let data = [0b00_01_10_11u8, 0b0_11_1_0000u8];
    let mut buf = Buffer::from_slice(&data);

    assert_eq!(buf.get_ns(5), 0);
    assert_eq!(buf.get_ns(5), 1);
    assert_eq!(buf.get_ns(5), 2);
    assert_eq!(buf.get_ns(5), 3);
    assert_eq!(buf.get_ns(5), 4);
}

/// byte_align() skips padding bits to reach the next byte boundary.
#[test]
fn test_buffer_byte_align() {
    let data = [0xFFu8, 0xAAu8];
    let mut buf = Buffer::from_slice(&data);

    buf.get_bits(3);
    assert!(!buf.is_byte_aligned());

    buf.byte_align();
    assert!(buf.is_byte_aligned());

    assert_eq!(buf.get_bits(8), 0xAA);
}

/// Parsing two consecutive OBUs: sequence header followed by temporal delimiter.
#[test]
fn test_parse_sequence_then_temporal_delimiter() {
    let mut stream = build_sequence_header_obu_640x480();
    stream.extend_from_slice(&[0x12u8, 0x00u8]); // append temporal delimiter

    let mut parser = ObuParser::default();
    let mut buf = Buffer::from_slice(&stream);

    let obu1 = parser
        .parse(&mut buf)
        .expect("sequence header should parse");
    assert!(matches!(obu1, Obu::SequenceHeader(_)));

    let obu2 = parser
        .parse(&mut buf)
        .expect("temporal delimiter should parse");
    assert!(matches!(obu2, Obu::TemporalDelimiter));
}

// ─────────────────────────────────────────────────────────────
// IVF file test (optional; requires an external fixture file)
// ─────────────────────────────────────────────────────────────

/// IVF file integration test.
///
/// IVF is a simple container for raw AV1 bitstreams:
/// - 32-byte file header: "DKIF" signature + version + codec + resolution + frame rate
/// - Repeated frame records: 12-byte frame header (size + PTS) + OBU data
///
/// This test reads the repository-local `DEMO.ivf` fixture.
#[test]
fn test_parse_ivf_file_if_available() {
    let test_file_path = "DEMO.ivf";

    if !std::path::Path::new(test_file_path).exists() {
        eprintln!(
            "Skipping IVF file test: {} not found.\n\
             Place DEMO.ivf at the repository root.",
            test_file_path
        );

        return;
    }

    let file_data = std::fs::read(&test_file_path).expect("cannot read test file");
    let ivf = IvfReader::new(&file_data).expect("test file should be a valid IVF container");
    let header = ivf.header();

    println!(
        "IVF file: codec={}, resolution={}x{}",
        header.codec_string(),
        header.width,
        header.height
    );

    let mut frame_count = 0usize;
    let mut seq_header_found = false;
    let mut parser = ObuParser::default();

    for ivf_frame in ivf.frames().take(10) {
        let ivf_frame = ivf_frame.expect("IVF frame should be well-formed");
        let mut buf = Buffer::from_slice(ivf_frame.data);
        loop {
            match parser.parse(&mut buf) {
                Ok(Obu::SequenceHeader(seq)) => {
                    println!(
                        "frame {}: SequenceHeader {}x{} profile={:?}",
                        ivf_frame.index, seq.max_frame_width, seq.max_frame_height, seq.seq_profile
                    );
                    seq_header_found = true;
                }
                Ok(Obu::Frame(frame)) => {
                    println!(
                        "frame {}: Frame type={:?} {}x{} Q={} tiles={}x{}",
                        ivf_frame.index,
                        frame.header.frame_type,
                        frame.header.frame_width,
                        frame.header.frame_height,
                        frame.header.quantization_params.base_q_idx,
                        frame.header.tile_info.tile_cols,
                        frame.header.tile_info.tile_rows,
                    );
                }
                Ok(Obu::FrameHeader(fh)) => {
                    println!(
                        "frame {}: FrameHeader type={:?} {}x{} Q={}",
                        ivf_frame.index,
                        fh.frame_type,
                        fh.frame_width,
                        fh.frame_height,
                        fh.quantization_params.base_q_idx,
                    );
                }
                Ok(Obu::TemporalDelimiter) | Ok(Obu::Drop) => {}
                Ok(other) => {
                    println!("frame {}: other OBU: {:?}", ivf_frame.index, other);
                }
                Err(e) => {
                    eprintln!("frame {}: parse error: {:?}", ivf_frame.index, e);
                    break;
                }
            }

            if buf.bytes_remaining() == 0 {
                break;
            }
        }

        frame_count += 1;
    }

    println!("parsed {} frames total", frame_count);
    assert!(
        seq_header_found,
        "should find a sequence header in the bitstream"
    );
    assert!(frame_count > 0, "should parse at least one frame");
}
