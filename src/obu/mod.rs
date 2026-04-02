/// AV1 Open Bitstream Unit (OBU) top-level parser.
///
/// An AV1 bitstream is a sequence of OBUs. Each OBU has a fixed header
/// structure that identifies its type. This module dispatches parsing to the
/// appropriate submodule and maintains cross-frame decoder context.
///
/// Reference: https://aomediacodec.github.io/av1-spec/#obu-syntax
pub mod frame;
pub mod frame_header;
pub mod metadata;
pub mod sequence_header;
pub mod tile_group;
pub mod tile_list;

use self::{
    frame::Frame,
    frame_header::{FrameHeader, FrameType},
    metadata::Metadata,
    sequence_header::SequenceHeader,
    tile_group::TileGroup,
    tile_list::TileList,
};

use crate::buffer::Buffer;

/// Number of reference frames that can be used for inter prediction.
pub const REFS_PER_FRAME: u8 = 7;

/// Number of reference frame types, including the intra type.
pub const TOTAL_REFS_PER_FRAME: u8 = 8;

/// Maximum width of a tile in luma samples.
pub const MAX_TILE_WIDTH: u16 = 4096;

/// Maximum area of a tile in luma samples.
pub const MAX_TILE_AREA: u32 = 4096 * 2304;

/// Maximum number of tile rows.
pub const MAX_TILE_ROWS: u8 = 64;

/// Maximum number of tile columns.
pub const MAX_TILE_COLS: u8 = 64;

/// Number of frames that can be stored for future reference.
pub const NUM_REF_FRAMES: u8 = 8;

/// Number of segments allowed in the segmentation map.
pub const MAX_SEGMENTS: u8 = 8;

/// Number of bits encoded for translational components of global motion models
/// used by `ROTZOOM` and `AFFINE`.
pub const GM_ABS_TRANS_BITS: u8 = 12;

/// Number of bits encoded for translational components of pure translation
/// global motion models.
pub const GM_ABS_TRANS_ONLY_BITS: u8 = 9;

/// Number of bits encoded for non-translational global motion components.
pub const GM_ABS_ALPHA_BITS: u8 = 12;

/// Number of fractional bits for non-translational warp model coefficients.
pub const GM_ALPHA_PREC_BITS: u8 = 15;

/// Number of fractional bits for translational warp model coefficients.
pub const GM_TRANS_PREC_BITS: u8 = 6;

/// Number of fractional bits used for pure translational warps.
pub const GM_TRANS_ONLY_PREC_BITS: u8 = 3;

/// Controls how self-guided restoration deltas are read.
pub const SGRPROJ_PRJ_SUBEXP_K: u8 = 4;

/// Value indicating that `allow_screen_content_tools` is explicitly coded.
pub const SELECT_SCREEN_CONTENT_TOOLS: u8 = 2;

/// Value indicating that `force_integer_mv` is explicitly coded.
pub const SELECT_INTEGER_MV: u8 = 2;

/// Smallest denominator used for super-resolution scaling.
pub const SUPERRES_DENOM_MIN: u8 = 9;

/// Number of bits sent to specify the super-resolution denominator.
pub const SUPERRES_DENOM_BITS: u8 = 3;

/// Numerator used for the super-resolution scaling ratio.
pub const SUPERRES_NUM: u8 = 8;

/// Internal precision of warped motion models.
pub const WARPEDMODEL_PREC_BITS: u8 = 16;

/// `primary_ref_frame` sentinel indicating there is no primary reference frame.
pub const PRIMARY_REF_NONE: u8 = 7;

// ─────────────────────────────────────────────────────────────
// OBU type
// ─────────────────────────────────────────────────────────────

/// OBU type identifiers.
///
/// AV1 spec Section 6.2.2 - obu_type semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObuType {
    /// Reserved value — should not appear in a conformant bitstream.
    Reserved(u8),
    /// Sequence header: global parameters for the entire sequence.
    SequenceHeader,
    /// Temporal delimiter: marks the start of a new temporal unit; empty payload.
    TemporalDelimiter,
    /// Frame header: per-frame coding parameters (without tile data).
    FrameHeader,
    /// Tile group: encoded data for a contiguous set of tiles.
    TileGroup,
    /// Metadata: HDR info, timecode, extensibility data, etc.
    Metadata,
    /// Frame: combined frame header + tile group (the common case).
    Frame,
    /// Redundant frame header: duplicate of the most recent FrameHeader OBU,
    /// used for error resilience.
    RedundantFrameHeader,
    /// Tile list: used in large-scale tile composition (e.g. LCEVC).
    TileList,
    /// Padding: alignment/filler bytes that decoders must ignore.
    Padding,
}

impl TryFrom<u8> for ObuType {
    type Error = ObuError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 | 9..=14 => Self::Reserved(value),
            1 => Self::SequenceHeader,
            2 => Self::TemporalDelimiter,
            3 => Self::FrameHeader,
            4 => Self::TileGroup,
            5 => Self::Metadata,
            6 => Self::Frame,
            7 => Self::RedundantFrameHeader,
            8 => Self::TileList,
            15 => Self::Padding,
            _ => return Err(ObuError::Unknown(ObuUnknownError::ObuHeaderType)),
        })
    }
}

// ─────────────────────────────────────────────────────────────
// OBU headers
// ─────────────────────────────────────────────────────────────

/// OBU extension header.
///
/// AV1 spec Section 5.3.3 - obu_extension_header().
/// Present when `obu_extension_flag` is set in the OBU header; used in
/// scalable (multi-layer) bitstreams to identify temporal and spatial layers.
#[derive(Debug, Clone, Copy)]
pub struct ObuHeaderExtension {
    /// Temporal layer ID (0–7). Lower values represent more base (lower frame-rate) layers.
    pub temporal_id: u8,
    /// Spatial layer ID (0–3). Lower values represent more base (lower resolution) layers.
    pub spatial_id: u8,
}

impl ObuHeaderExtension {
    pub fn decode(buf: &mut Buffer<'_>) -> Result<Self, ObuError> {
        // temporal_id	f(3)
        let temporal_id = buf.get_bits(3) as u8;
        // spatial_id	f(2)
        let spatial_id = buf.get_bits(2) as u8;
        // extension_header_reserved_3bits	f(3) — must be 0, decoder ignores
        buf.seek_bits(3);

        Ok(Self {
            temporal_id,
            spatial_id,
        })
    }
}

/// OBU header.
///
/// AV1 spec Section 5.3.2 - obu_header().
/// Every OBU begins with this header, carrying the type, a size-field flag,
/// and an optional extension header for scalable bitstreams.
#[derive(Debug, Clone, Copy)]
pub struct ObuHeader {
    /// OBU type.
    pub r#type: ObuType,
    /// Whether an `obu_size` field follows the header.
    pub has_size: bool,
    /// Optional extension header (present in scalable bitstreams).
    pub extension: Option<ObuHeaderExtension>,
}

impl ObuHeader {
    pub fn decode(buf: &mut Buffer<'_>) -> Result<Self, ObuError> {
        // obu_forbidden_bit	f(1) — must be 0
        buf.seek_bits(1);
        // obu_type	f(4)
        let r#type = ObuType::try_from(buf.get_bits(4) as u8)?;
        // obu_extension_flag	f(1)
        let obu_extension_flag = buf.get_bit();
        // obu_has_size_field	f(1)
        let has_size = buf.get_bit();
        // obu_reserved_1bit	f(1) — must be 0
        buf.seek_bits(1);

        let extension = if obu_extension_flag {
            Some(ObuHeaderExtension::decode(buf.as_mut())?)
        } else {
            None
        };

        Ok(Self {
            r#type,
            has_size,
            extension,
        })
    }
}

// ─────────────────────────────────────────────────────────────
// Parse result
// ─────────────────────────────────────────────────────────────

/// Result of parsing a single OBU.
///
/// Returned by each call to [`ObuParser::parse`].
#[derive(Debug)]
pub enum Obu {
    /// Sequence header with global encoding parameters.
    SequenceHeader(SequenceHeader),
    /// Complete frame (header + tile group).
    Frame(Frame),
    /// Standalone frame header (usually followed by a TileGroup OBU).
    FrameHeader(FrameHeader),
    /// Standalone tile group (follows a FrameHeader OBU).
    TileGroup(TileGroup),
    /// Metadata (HDR, timecode, etc.).
    Metadata(Metadata),
    /// Tile list.
    TileList(TileList),
    /// Temporal delimiter — signals the start of a new temporal unit.
    TemporalDelimiter,
    /// Redundant frame header — same content as the most recent FrameHeader OBU.
    RedundantFrameHeader,
    /// OBU that does not belong to the current operating point; caller may discard it.
    Drop,
}

// ─────────────────────────────────────────────────────────────
// Decoder context
// ─────────────────────────────────────────────────────────────

/// Cross-frame decoder context maintained by [`ObuParser`].
///
/// Persistent state required to correctly parse fields that depend on
/// information from previous OBUs (e.g. reference frame sizes, order hints).
#[derive(Debug)]
pub struct ObuContext {
    /// Sequence header decoded from the most recent SequenceHeader OBU.
    pub sequence_header: Option<SequenceHeader>,
    /// Extension header of the OBU currently being parsed (for layer filtering).
    pub obu_header_extension: Option<ObuHeaderExtension>,

    // ── Plane / bit-depth info ────────────────────────────────
    /// Number of color planes (1 = monochrome, 3 = YUV).
    pub num_planes: u8,
    /// Sample bit depth (8, 10, or 12).
    pub bit_depth: u8,

    // ── Frame header state ────────────────────────────────────
    /// Whether a frame header has been seen for the current temporal unit.
    pub seen_frame_header: bool,
    /// Whether the current frame is intra-only (KeyFrame or IntraOnlyFrame).
    pub frame_is_intra: bool,

    // ── Frame dimensions ──────────────────────────────────────
    /// Width of the encoded frame in luma samples (may be superres-downscaled).
    pub frame_width: u16,
    /// Height of the encoded frame in luma samples.
    pub frame_height: u16,
    /// Width after superres upscaling (== frame_width when superres is off).
    pub upscaled_width: u16,
    /// Superres denominator (actual scale = SUPERRES_NUM / superres_denom).
    pub superres_denom: u8,
    /// Number of 4×4 MI columns (= 2 × ⌈frame_width / 8⌉).
    pub mi_cols: u32,
    /// Number of 4×4 MI rows.
    pub mi_rows: u32,
    /// Suggested display/render width.
    pub render_width: u16,
    /// Suggested display/render height.
    pub render_height: u16,

    // ── Order hints ───────────────────────────────────────────
    /// Display order hint for the current frame.
    pub order_hint: u32,
    /// Number of bits used for order_hint fields (0 = order_hint disabled).
    pub order_hint_bits: usize,

    // ── Operating point ───────────────────────────────────────
    /// Index of the target operating point (selects which quality/resolution layer to decode).
    pub operating_point: usize,
    /// IDC bitmask for the current operating point (temporal/spatial layer mask).
    pub operating_point_idc: u16,

    // ── Reference frame buffer state ─────────────────────────
    /// Frame type stored in each of the NUM_REF_FRAMES reference buffer slots.
    pub ref_frame_type: Vec<FrameType>,
    /// Validity flag for each reference buffer slot (false = invalidated).
    pub ref_frame_marking: Vec<bool>,
    /// Order hint stored in each reference buffer slot.
    pub ref_order_hint: Vec<u32>,
    /// Upscaled width of the frame in each reference buffer slot.
    pub ref_upscaled_width: Vec<u16>,
    /// Frame height of the frame in each reference buffer slot.
    pub ref_frame_height: Vec<u16>,
    /// Render width of the frame in each reference buffer slot.
    pub ref_render_width: Vec<u16>,
    /// Render height of the frame in each reference buffer slot.
    pub ref_render_height: Vec<u16>,
    /// Mapping from the REFS_PER_FRAME reference types to buffer slot indices.
    pub ref_frame_idx: [u8; REFS_PER_FRAME as usize],

    // ── Inter-frame state ─────────────────────────────────────
    /// Delta frame ID for reference frame validity checking.
    pub delta_frame_id: u32,

    // ── Global motion (carried across frames) ────────────────
    /// Global motion parameters from the previous frame, used for delta coding.
    pub prev_gm_params: [[i32; 6]; REFS_PER_FRAME as usize],

    // ── Tile state ────────────────────────────────────────────
    /// Index of the tile currently being processed.
    pub tile_num: usize,
}

impl Default for ObuContext {
    fn default() -> Self {
        // The spec initialises all reference frame slots as KeyFrame type.
        let ref_frame_type = vec![FrameType::KeyFrame; NUM_REF_FRAMES as usize];
        let ref_frame_marking = vec![false; NUM_REF_FRAMES as usize];
        let ref_order_hint = vec![0u32; NUM_REF_FRAMES as usize];
        let ref_upscaled_width = vec![0u16; NUM_REF_FRAMES as usize];
        let ref_frame_height = vec![0u16; NUM_REF_FRAMES as usize];
        let ref_render_width = vec![0u16; NUM_REF_FRAMES as usize];
        let ref_render_height = vec![0u16; NUM_REF_FRAMES as usize];

        Self {
            sequence_header: None,
            obu_header_extension: None,
            num_planes: 3,
            bit_depth: 8,
            seen_frame_header: false,
            frame_is_intra: false,
            frame_width: 0,
            frame_height: 0,
            upscaled_width: 0,
            superres_denom: 8, // SUPERRES_NUM — no superres by default
            mi_cols: 0,
            mi_rows: 0,
            render_width: 0,
            render_height: 0,
            order_hint: 0,
            order_hint_bits: 0,
            operating_point: 0,
            operating_point_idc: 0,
            ref_frame_type,
            ref_frame_marking,
            ref_order_hint,
            ref_upscaled_width,
            ref_frame_height,
            ref_render_width,
            ref_render_height,
            ref_frame_idx: [0; REFS_PER_FRAME as usize],
            delta_frame_id: 0,
            prev_gm_params: [[0i32; 6]; REFS_PER_FRAME as usize],
            tile_num: 0,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// OBU parser
// ─────────────────────────────────────────────────────────────

/// Streaming AV1 OBU parser.
///
/// Maintains cross-frame [`ObuContext`] state. Call [`parse`] in a loop
/// to consume one OBU at a time from the bitstream.
///
/// ```no_run
/// use av1_obu_parser::{buffer::Buffer, obu::ObuParser};
/// let data: Vec<u8> = vec![]; // AV1 bitstream bytes
/// let mut parser = ObuParser::default();
/// let mut buf = Buffer::from_slice(&data);
/// loop {
///     match parser.parse(&mut buf) {
///         Ok(obu) => println!("{:?}", obu),
///         Err(e) => break,
///     }
/// }
/// ```
#[derive(Default)]
pub struct ObuParser {
    pub ctx: ObuContext,
}

impl ObuParser {
    /// Parse the next OBU from the bitstream.
    ///
    /// AV1 spec Section 5.3 - open_bitstream_unit().
    pub fn parse(&mut self, buf: &mut Buffer) -> Result<Obu, ObuError> {
        // ── OBU header ────────────────────────────────────────
        let header = ObuHeader::decode(buf.as_mut())?;

        // Store the extension header so frame-header parsing can use it for
        // operating-point layer filtering.
        self.ctx.obu_header_extension = header.extension;

        // ── OBU size ──────────────────────────────────────────
        let _size = if header.has_size {
            // obu_size	leb128() — byte length of the OBU payload
            Some(buf.get_leb128() as usize)
        } else {
            None
        };

        // ── Operating-point filtering ─────────────────────────
        // For OBUs other than SequenceHeader and TemporalDelimiter, discard
        // the OBU if it does not belong to the current operating point.
        if header.r#type != ObuType::SequenceHeader
            && header.r#type != ObuType::TemporalDelimiter
            && self.ctx.operating_point_idc != 0
        {
            if let Some(ext) = header.extension {
                let in_temporal_layer = (self.ctx.operating_point_idc >> ext.temporal_id) & 1;
                let in_spatial_layer = (self.ctx.operating_point_idc >> (ext.spatial_id + 8)) & 1;
                if in_temporal_layer == 0 || in_spatial_layer == 0 {
                    return Ok(Obu::Drop);
                }
            }
        }

        // ── Dispatch by OBU type ──────────────────────────────
        // Record the payload start so we can advance to the declared end later.
        let payload_start_bytes = buf.bytes_consumed();

        let result = match header.r#type {
            ObuType::SequenceHeader => {
                let seq = SequenceHeader::decode(&mut self.ctx, buf)?;

                // Store the sequence header so frame-header parsing can use it for
                // operating-point layer filtering.
                self.ctx.sequence_header = Some(seq.clone());

                Obu::SequenceHeader(seq)
            }
            ObuType::TemporalDelimiter => {
                // Empty payload; reset seen_frame_header for the new temporal unit.
                self.ctx.seen_frame_header = false;
                Obu::TemporalDelimiter
            }
            ObuType::FrameHeader => Obu::FrameHeader(FrameHeader::decode(&mut self.ctx, buf)?),
            ObuType::Frame => Obu::Frame(Frame::decode(&mut self.ctx, buf)?),
            ObuType::TileGroup => {
                // A standalone TileGroup OBU requires the tile layout from the
                // preceding FrameHeader OBU.
                // TODO: store the most recent frame header tile_info in ctx so
                // we can parse the tile group properly here.
                Obu::TileGroup(TileGroup::empty())
            }
            ObuType::RedundantFrameHeader => {
                // Identical to the most recent FrameHeader OBU; used only for
                // error resilience. Safe to ignore for analysis.
                Obu::RedundantFrameHeader
            }
            ObuType::Metadata => Obu::Metadata(Metadata::decode(buf)?),
            ObuType::TileList => Obu::TileList(TileList::decode(buf)),
            ObuType::Padding | ObuType::Reserved(_) => {
                // Padding and reserved OBUs carry no meaningful data.
                Obu::Drop
            }
        };

        // ── Advance to OBU boundary ───────────────────────────
        // AV1 spec requires each OBU payload to end with trailing_bits() which
        // pads the stream to a byte boundary (a 1-bit followed by zero bits).
        // We enforce the OBU boundary using the declared size when available,
        // otherwise we just byte-align to skip the trailing padding.
        if let Some(size) = _size {
            let bytes_consumed_in_payload =
                buf.bytes_consumed().saturating_sub(payload_start_bytes);
            if bytes_consumed_in_payload < size {
                // Skip any declared bytes that were not consumed (e.g. Padding OBU).
                let remaining = size - bytes_consumed_in_payload;
                buf.seek_bits(remaining * 8);
            } else {
                // All declared bytes consumed; skip any trailing padding bits.
                buf.byte_align();
            }
        } else {
            buf.byte_align();
        }

        Ok(result)
    }
}

// ─────────────────────────────────────────────────────────────
// Error types
// ─────────────────────────────────────────────────────────────

/// Specific category of an unknown value encountered during parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObuUnknownError {
    ObuHeaderType,
    Profile,
    ColorPrimaries,
    TransferCharacteristics,
    MatrixCoefficients,
    ChromaSamplePosition,
    MetadataType,
    ScalabilityModeIdc,
    FrameType,
    InterpolationFilter,
    FrameTypeRefIndex,
}

/// OBU parsing error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObuError {
    /// An unknown or unsupported value was encountered in the bitstream.
    Unknown(ObuUnknownError),
    /// A frame header was encountered before any sequence header.
    NotFoundSequenceHeader,
}

impl std::error::Error for ObuError {}

impl std::fmt::Display for ObuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObuError::Unknown(e) => write!(f, "Unknown bitstream value: {:?}", e),
            ObuError::NotFoundSequenceHeader => {
                write!(f, "Frame header encountered before sequence header")
            }
        }
    }
}
