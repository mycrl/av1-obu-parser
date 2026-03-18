/// AV1 Frame Header — complete parsing implementation.
///
/// Corresponds to AV1 spec Section 5.9 - uncompressed_header() and its
/// sub-functions. This module acts as a bitstream analysis tool: it parses
/// and records all key information in the frame header but does not perform
/// the CDF updates, reference frame buffer management, or other runtime state
/// maintenance required by a full decoder.
use super::{Buffer, ObuContext, ObuError, ObuUnknownError};

use crate::constants::{
    MAX_SEGMENTS, MAX_TILE_AREA, MAX_TILE_COLS, MAX_TILE_ROWS, MAX_TILE_WIDTH,
    NUM_REF_FRAMES, PRIMARY_REF_NONE, REFS_PER_FRAME, SELECT_INTEGER_MV,
    SELECT_SCREEN_CONTENT_TOOLS, SGRPROJ_PRJ_SUBEXP_K, SUPERRES_DENOM_BITS, SUPERRES_DENOM_MIN,
    SUPERRES_NUM, TOTAL_REFS_PER_FRAME,
};

// ─────────────────────────────────────────────────────────────
// Core enumerations
// ─────────────────────────────────────────────────────────────

/// AV1 frame type.
///
/// AV1 spec Section 6.8.2 - frame_type semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    /// Key frame: does not reference any previous frame; serves as a random access point.
    KeyFrame,
    /// Inter frame: uses reference frames for motion-compensated prediction.
    InterFrame,
    /// Intra-only frame: uses only intra prediction but is not a key frame.
    IntraOnlyFrame,
    /// Switch frame: can replace reference frame buffers.
    SwitchFrame,
}

impl TryFrom<u8> for FrameType {
    type Error = ObuError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::KeyFrame,
            1 => Self::InterFrame,
            2 => Self::IntraOnlyFrame,
            3 => Self::SwitchFrame,
            _ => return Err(ObuError::Unknown(ObuUnknownError::FrameType)),
        })
    }
}

/// Interpolation filter type used in inter prediction.
///
/// AV1 spec Section 6.8.33 - interpolation_filter semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterpolationFilter {
    /// Standard 8-tap filter.
    Eighttap,
    /// Smooth 8-tap filter.
    EighttapSmooth,
    /// Sharp 8-tap filter.
    EighttapSharp,
    /// Bilinear filter.
    Bilinear,
    /// Switchable — encoder selects per block.
    Switchable,
}

impl TryFrom<u8> for InterpolationFilter {
    type Error = ObuError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::Eighttap,
            1 => Self::EighttapSmooth,
            2 => Self::EighttapSharp,
            3 => Self::Bilinear,
            4 => Self::Switchable,
            _ => return Err(ObuError::Unknown(ObuUnknownError::InterpolationFilter)),
        })
    }
}

/// Transform size selection mode.
///
/// AV1 spec Section 6.8.38 - TxMode semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxMode {
    /// Only 4×4 transforms are used.
    Only4x4,
    /// The largest transform allowed by the block size is used.
    Largest,
    /// Transform size is signalled in the bitstream per block.
    Select,
}

/// Global motion model type.
///
/// AV1 spec Section 6.8.39 - GmType semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GmType {
    /// Identity transform (no motion).
    Identity,
    /// Pure translation.
    Translation,
    /// Rotation + uniform scale + translation (4 degrees of freedom).
    RotZoom,
    /// Full affine transform (6 degrees of freedom).
    Affine,
}

// ─────────────────────────────────────────────────────────────
// Temporal point info
// ─────────────────────────────────────────────────────────────

/// Frame presentation time information.
///
/// AV1 spec Section 5.9.2 - temporal_point_info().
#[derive(Debug, Clone)]
pub struct TemporalPointInfo {
    /// Frame presentation time in display_tick units.
    pub frame_presentation_time: u32,
}

impl TemporalPointInfo {
    pub fn decode(buf: &mut Buffer, frame_presentation_time_length: usize) -> Self {
        // frame_presentation_time	f(n)
        Self {
            frame_presentation_time: buf.get_bits(frame_presentation_time_length),
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Frame size helpers
// ─────────────────────────────────────────────────────────────

/// Compute the number of MI (4×4 unit) columns and rows for the current frame.
///
/// mi_cols and mi_rows count 4×4 units at double density to align with
/// superblock boundaries.
#[inline]
pub fn compute_image_size(ctx: &mut ObuContext) {
    ctx.mi_cols = 2 * ((ctx.frame_width + 7) >> 3) as u32;
    ctx.mi_rows = 2 * ((ctx.frame_height + 7) >> 3) as u32;
}

/// Parse frame_size().
///
/// AV1 spec Section 5.9.8 - frame_size().
/// Reads the coded frame dimensions when frame_size_override is set;
/// otherwise uses the maximum dimensions from the sequence header.
#[inline]
pub fn frame_size(ctx: &mut ObuContext, frame_size_override: bool, buf: &mut Buffer) {
    let sequence_header = ctx
        .sequence_header
        .as_ref()
        .expect("sequence_header must be present before frame_size()");

    let (width, height) = if frame_size_override {
        (
            // frame_width_minus_1	f(n)
            buf.get_bits(sequence_header.frame_width_bits as usize) as u16 + 1,
            // frame_height_minus_1	f(n)
            buf.get_bits(sequence_header.frame_height_bits as usize) as u16 + 1,
        )
    } else {
        (
            sequence_header.max_frame_width,
            sequence_header.max_frame_height,
        )
    };

    ctx.frame_width = width;
    ctx.frame_height = height;

    superres_params(ctx, buf);
    compute_image_size(ctx);
}

/// Parse superres_params().
///
/// AV1 spec Section 5.9.10 - superres_params().
/// Super-resolution allows the frame to be coded at reduced resolution and
/// then upsampled. The scale factor is SUPERRES_NUM / superres_denom (8 / d).
#[inline]
pub fn superres_params(ctx: &mut ObuContext, buf: &mut Buffer) {
    let sequence_header = ctx
        .sequence_header
        .as_ref()
        .expect("sequence_header must be present before superres_params()");

    let use_superres = if sequence_header.enable_superres {
        // use_superres	f(1)
        buf.get_bit()
    } else {
        false
    };

    ctx.superres_denom = if use_superres {
        // coded_denom	f(SUPERRES_DENOM_BITS)
        // actual denominator = coded_denom + SUPERRES_DENOM_MIN
        let coded_denom = buf.get_bits(SUPERRES_DENOM_BITS as usize) as u8;
        coded_denom + SUPERRES_DENOM_MIN
    } else {
        SUPERRES_NUM // denominator == numerator → 1:1 scale
    };

    // UpscaledWidth is the intended output width; FrameWidth is the coded width.
    ctx.upscaled_width = ctx.frame_width;
    ctx.frame_width = (ctx.upscaled_width as u32 * SUPERRES_NUM as u32
        + ctx.superres_denom as u32 / 2)
        as u16
        / ctx.superres_denom as u16;
}

/// Parse render_size().
///
/// AV1 spec Section 5.9.11 - render_size().
/// The render size is the suggested display size; it may differ from the
/// coded frame size (e.g. for letterboxing or anamorphic video).
#[inline]
pub fn render_size(ctx: &mut ObuContext, buf: &mut Buffer) {
    // render_and_frame_size_different	f(1)
    let render_and_frame_size_different = buf.get_bit();
    let (width, height) = if render_and_frame_size_different {
        (
            // render_width_minus_1	f(16)
            buf.get_bits(16) as u16 + 1,
            // render_height_minus_1	f(16)
            buf.get_bits(16) as u16 + 1,
        )
    } else {
        (ctx.upscaled_width, ctx.frame_height)
    };

    ctx.render_width = width;
    ctx.render_height = height;
}

/// Parse frame_size_with_refs().
///
/// AV1 spec Section 5.9.9 - frame_size_with_refs().
/// For inter frames the frame dimensions may be inherited from a reference
/// frame to save bits.
#[inline]
pub fn frame_size_with_refs(
    ctx: &mut ObuContext,
    frame_size_override: bool,
    ref_frame_idx: &[u8; REFS_PER_FRAME as usize],
    buf: &mut Buffer,
) {
    let mut found_ref = false;

    for i in 0..REFS_PER_FRAME as usize {
        // found_ref	f(1)
        found_ref = buf.get_bit();

        if found_ref {
            let ref_idx = ref_frame_idx[i] as usize;
            ctx.upscaled_width = ctx.ref_upscaled_width[ref_idx];
            ctx.frame_width = ctx.upscaled_width;
            ctx.frame_height = ctx.ref_frame_height[ref_idx];
            ctx.render_width = ctx.ref_render_width[ref_idx];
            ctx.render_height = ctx.ref_render_height[ref_idx];
            break;
        }
    }

    if !found_ref {
        frame_size(ctx, frame_size_override, buf);
        render_size(ctx, buf);
    } else {
        superres_params(ctx, buf);
        compute_image_size(ctx);
    }
}

// ─────────────────────────────────────────────────────────────
// tile_log2 helper
// ─────────────────────────────────────────────────────────────

/// Smallest power-of-two exponent k such that (blk_size << k) >= target.
///
/// tile_log2(blkSize, target) from the AV1 spec.
pub fn tile_log2(blk_size: u32, target: u32) -> usize {
    let mut k = 0usize;
    while (blk_size << k) < target {
        k += 1;
    }
    k
}

// ─────────────────────────────────────────────────────────────
// Tile info
// ─────────────────────────────────────────────────────────────

/// Tile grid layout.
///
/// AV1 spec Section 5.9.15 - tile_info().
/// Tiles are rectangular partitions of a frame that can be decoded
/// independently. All positions are expressed in MI (4×4) units.
#[derive(Debug, Clone)]
pub struct TileInfo {
    /// Whether all tiles have uniform width and height.
    pub uniform_tile_spacing_flag: bool,
    /// log2 of the number of tile columns.
    pub tile_cols_log2: usize,
    /// log2 of the number of tile rows.
    pub tile_rows_log2: usize,
    /// Number of tile columns.
    pub tile_cols: usize,
    /// Number of tile rows.
    pub tile_rows: usize,
    /// MI-unit start position of each tile column.
    pub mi_col_starts: Vec<u32>,
    /// MI-unit start position of each tile row.
    pub mi_row_starts: Vec<u32>,
    /// Tile ID used for context model updates.
    pub context_update_tile_id: u32,
    /// Number of bytes used to encode each tile's size field (1–4).
    pub tile_size_bytes: usize,
}

impl TileInfo {
    pub fn decode(ctx: &ObuContext, buf: &mut Buffer) -> Self {
        let seq = ctx.sequence_header.as_ref().unwrap();

        // Compute superblock counts based on superblock size.
        let (sb_cols, sb_rows, sb_shift) = if seq.use_128x128_superblock {
            let cols = (ctx.mi_cols + 31) >> 5;
            let rows = (ctx.mi_rows + 31) >> 5;
            (cols, rows, 5usize)
        } else {
            let cols = (ctx.mi_cols + 15) >> 4;
            let rows = (ctx.mi_rows + 15) >> 4;
            (cols, rows, 4usize)
        };

        // sb_size = log2 of superblock size in MI units
        let sb_size = sb_shift + 2;

        let sb_max_tile_cols = (MAX_TILE_WIDTH as u32) >> sb_size;
        let sb_max_tile_area = (MAX_TILE_AREA as u64) >> (2 * sb_size);

        let min_log2_tile_cols = tile_log2(sb_max_tile_cols, sb_cols);
        let max_log2_tile_cols = tile_log2(1, sb_cols.min(MAX_TILE_COLS as u32));
        let max_log2_tile_rows = tile_log2(1, sb_rows.min(MAX_TILE_ROWS as u32));
        let min_log2_tiles = min_log2_tile_cols
            .max(tile_log2(sb_max_tile_area as u32, sb_cols * sb_rows));

        // uniform_tile_spacing_flag	f(1)
        let uniform_tile_spacing_flag = buf.get_bit();

        let mut mi_col_starts = Vec::new();
        let mut mi_row_starts = Vec::new();
        let mut tile_cols_log2 = min_log2_tile_cols;
        let mut tile_rows_log2;
        let tile_cols;
        let tile_rows;

        if uniform_tile_spacing_flag {
            // Uniform tile spacing: increment log2 until reaching max.
            while tile_cols_log2 < max_log2_tile_cols {
                // increment_tile_cols_log2	f(1)
                if buf.get_bit() {
                    tile_cols_log2 += 1;
                } else {
                    break;
                }
            }

            let tile_width_sb =
                (sb_cols + (1 << tile_cols_log2) - 1) >> tile_cols_log2;
            let mut start_sb = 0u32;
            let mut i = 0usize;
            while start_sb < sb_cols {
                mi_col_starts.push(start_sb << sb_shift);
                i += 1;
                start_sb += tile_width_sb;
            }
            mi_col_starts.push(ctx.mi_cols);
            tile_cols = i;

            let min_log2_tile_rows =
                min_log2_tiles.saturating_sub(tile_cols_log2);
            tile_rows_log2 = min_log2_tile_rows;
            while tile_rows_log2 < max_log2_tile_rows {
                // increment_tile_rows_log2	f(1)
                if buf.get_bit() {
                    tile_rows_log2 += 1;
                } else {
                    break;
                }
            }

            let tile_height_sb =
                (sb_rows + (1 << tile_rows_log2) - 1) >> tile_rows_log2;
            let mut start_sb = 0u32;
            let mut i = 0usize;
            while start_sb < sb_rows {
                mi_row_starts.push(start_sb << sb_shift);
                i += 1;
                start_sb += tile_height_sb;
            }
            mi_row_starts.push(ctx.mi_rows);
            tile_rows = i;
        } else {
            // Non-uniform tile spacing: each tile width is individually coded.
            let mut widest_tile_sb = 0u32;
            let mut start_sb = 0u32;
            let mut i = 0usize;
            while start_sb < sb_cols {
                mi_col_starts.push(start_sb << sb_shift);
                let max_width = (sb_cols - start_sb).min(sb_max_tile_cols);
                // width_in_sbs_minus_1	ns(maxWidth)
                let width_in_sbs = buf.get_ns(max_width) + 1;
                widest_tile_sb = widest_tile_sb.max(width_in_sbs);
                start_sb += width_in_sbs;
                i += 1;
            }
            mi_col_starts.push(ctx.mi_cols);
            tile_cols = i;
            tile_cols_log2 = tile_log2(1, tile_cols as u32);

            let max_tile_area_sb = if min_log2_tiles > 0 {
                (sb_cols * sb_rows) >> min_log2_tiles
            } else {
                sb_cols * sb_rows
            };
            let max_tile_height_sb = (max_tile_area_sb / widest_tile_sb.max(1)).max(1);

            let mut start_sb = 0u32;
            let mut i = 0usize;
            while start_sb < sb_rows {
                mi_row_starts.push(start_sb << sb_shift);
                let max_height = (sb_rows - start_sb).min(max_tile_height_sb);
                // height_in_sbs_minus_1	ns(maxHeight)
                let height_in_sbs = buf.get_ns(max_height) + 1;
                start_sb += height_in_sbs;
                i += 1;
            }
            mi_row_starts.push(ctx.mi_rows);
            tile_rows = i;
            tile_rows_log2 = tile_log2(1, tile_rows as u32);
        }

        let num_tiles = tile_cols * tile_rows;

        let context_update_tile_id = if num_tiles > 1 {
            // context_update_tile_id	f(TileColsLog2 + TileRowsLog2)
            buf.get_bits(tile_cols_log2 + tile_rows_log2)
        } else {
            0
        };

        // tile_size_bytes_minus_1	f(2)
        let tile_size_bytes = if num_tiles > 1 {
            buf.get_bits(2) as usize + 1
        } else {
            4
        };

        Self {
            uniform_tile_spacing_flag,
            tile_cols_log2,
            tile_rows_log2,
            tile_cols,
            tile_rows,
            mi_col_starts,
            mi_row_starts,
            context_update_tile_id,
            tile_size_bytes,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Quantization parameters
// ─────────────────────────────────────────────────────────────

/// Quantization parameters.
///
/// AV1 spec Section 5.9.12 - quantization_params().
/// Quantization is the core of lossy compression; base_q_idx controls
/// the overall quality level (0 = lossless, 63 = lowest quality).
#[derive(Debug, Clone)]
pub struct QuantizationParams {
    /// Base quantization index (0 = lossless, up to 255).
    pub base_q_idx: u8,
    /// Delta-Q for the Y-plane DC component.
    pub delta_q_y_dc: i32,
    /// Whether U and V planes use independent delta-Q values.
    pub diff_uv_delta: bool,
    /// Delta-Q for the U-plane DC component.
    pub delta_q_u_dc: i32,
    /// Delta-Q for the U-plane AC component.
    pub delta_q_u_ac: i32,
    /// Delta-Q for the V-plane DC component.
    pub delta_q_v_dc: i32,
    /// Delta-Q for the V-plane AC component.
    pub delta_q_v_ac: i32,
    /// Whether quantization matrices (QM) are used.
    pub using_qmatrix: bool,
    /// QM level for the Y plane (0–15); 0xff = QM disabled.
    pub qm_y: u8,
    /// QM level for the U plane.
    pub qm_u: u8,
    /// QM level for the V plane.
    pub qm_v: u8,
}

/// Read a delta-Q field (delta_coded f(1) + delta_q su(7)).
///
/// AV1 spec Section 5.9.12 - read_delta_q().
fn read_delta_q(buf: &mut Buffer) -> i32 {
    // delta_coded	f(1)
    if buf.get_bit() {
        // delta_q	su(7)
        buf.get_su(7)
    } else {
        0
    }
}

impl QuantizationParams {
    pub fn decode(ctx: &ObuContext, buf: &mut Buffer) -> Self {
        // base_q_idx	f(8)
        let base_q_idx = buf.get_bits(8) as u8;

        let delta_q_y_dc = read_delta_q(buf);

        let num_planes = ctx.num_planes;
        let mut diff_uv_delta = false;
        let mut delta_q_u_dc = 0i32;
        let mut delta_q_u_ac = 0i32;
        let mut delta_q_v_dc = 0i32;
        let mut delta_q_v_ac = 0i32;

        if num_planes > 1 {
            let seq = ctx.sequence_header.as_ref().unwrap();
            if seq.color_config.separate_uv_delta_q {
                // diff_uv_delta	f(1)
                diff_uv_delta = buf.get_bit();
            }
            delta_q_u_dc = read_delta_q(buf);
            delta_q_u_ac = read_delta_q(buf);
            if diff_uv_delta {
                // V-plane delta-Q is independently coded when diff_uv_delta is set.
                delta_q_v_dc = read_delta_q(buf);
                delta_q_v_ac = read_delta_q(buf);
            } else {
                delta_q_v_dc = delta_q_u_dc;
                delta_q_v_ac = delta_q_u_ac;
            }
        }

        // using_qmatrix	f(1)
        let using_qmatrix = buf.get_bit();
        let (qm_y, qm_u, qm_v) = if using_qmatrix {
            // qm_y	f(4)
            let qm_y = buf.get_bits(4) as u8;
            // qm_u	f(4)
            let qm_u = buf.get_bits(4) as u8;
            let qm_v = if ctx
                .sequence_header
                .as_ref()
                .unwrap()
                .color_config
                .separate_uv_delta_q
            {
                // qm_v	f(4)
                buf.get_bits(4) as u8
            } else {
                qm_u // qm_v mirrors qm_u when separate_uv_delta_q is false
            };
            (qm_y, qm_u, qm_v)
        } else {
            (0xff, 0xff, 0xff) // 0xff sentinel: QM not in use
        };

        Self {
            base_q_idx,
            delta_q_y_dc,
            diff_uv_delta,
            delta_q_u_dc,
            delta_q_u_ac,
            delta_q_v_dc,
            delta_q_v_ac,
            using_qmatrix,
            qm_y,
            qm_u,
            qm_v,
        }
    }

    /// Returns true if all quantization parameters correspond to lossless coding.
    pub fn is_lossless(&self) -> bool {
        self.base_q_idx == 0
            && self.delta_q_y_dc == 0
            && self.delta_q_u_dc == 0
            && self.delta_q_u_ac == 0
            && self.delta_q_v_dc == 0
            && self.delta_q_v_ac == 0
    }
}

// ─────────────────────────────────────────────────────────────
// Segmentation parameters
// ─────────────────────────────────────────────────────────────

/// A single entry in the segment feature table.
///
/// Segmentation allows a frame to be divided into regions with different
/// quantization, loop filter levels, and other per-region parameters.
#[derive(Debug, Clone, Default)]
pub struct SegmentFeature {
    /// Whether this feature is enabled for the segment.
    pub enabled: bool,
    /// Feature value (quantization delta, loop filter delta, etc.).
    pub value: i16,
}

/// Segmentation parameters.
///
/// AV1 spec Section 5.9.14 - segmentation_params().
#[derive(Debug, Clone)]
pub struct SegmentationParams {
    /// Whether segmentation is enabled.
    pub segmentation_enabled: bool,
    /// Whether the segmentation map is updated (typically false for inter frames
    /// to reuse the previous frame's map).
    pub segmentation_update_map: bool,
    /// Whether the segmentation map is updated using temporal prediction.
    pub segmentation_temporal_update: bool,
    /// Whether segment feature data is updated.
    pub segmentation_update_data: bool,
    /// Feature table: [MAX_SEGMENTS=8][SEG_LVL_MAX=8].
    pub features: [[SegmentFeature; 8]; 8],
}

// Bit widths and signedness for each segment feature index:
// SEG_LVL_ALT_Q=8-bit signed, SEG_LVL_ALT_LF_*=6-bit signed,
// SEG_LVL_REF_FRAME=3-bit unsigned, SEG_LVL_SKIP/GLOBALMV=0-bit flag.
const FEATURE_BITS: [u8; 8] = [8, 6, 6, 6, 6, 3, 0, 0];
const FEATURE_SIGNED: [bool; 8] = [true, true, true, true, true, false, false, false];
const FEATURE_MAX: [i16; 8] = [255, 63, 63, 63, 63, 7, 0, 0];

impl SegmentationParams {
    pub fn decode(buf: &mut Buffer, frame_is_intra: bool, primary_ref_frame: u8) -> Self {
        // segmentation_enabled	f(1)
        let segmentation_enabled = buf.get_bit();

        let mut segmentation_update_map = false;
        let mut segmentation_temporal_update = false;
        let mut segmentation_update_data = false;
        let mut features = core::array::from_fn(|_| core::array::from_fn(|_| SegmentFeature::default()));

        if segmentation_enabled {
            if primary_ref_frame == PRIMARY_REF_NONE {
                // No primary reference frame: map and data must be fully refreshed.
                segmentation_update_map = true;
                segmentation_temporal_update = false;
                segmentation_update_data = true;
            } else {
                if !frame_is_intra {
                    // segmentation_update_map	f(1)
                    segmentation_update_map = buf.get_bit();
                    if segmentation_update_map {
                        // segmentation_temporal_update	f(1)
                        segmentation_temporal_update = buf.get_bit();
                    }
                }
                // segmentation_update_data	f(1)
                segmentation_update_data = buf.get_bit();
            }

            if segmentation_update_data {
                for i in 0..MAX_SEGMENTS as usize {
                    for j in 0..8usize {
                        // feature_enabled	f(1)
                        let enabled = buf.get_bit();
                        let value = if enabled && FEATURE_BITS[j] > 0 {
                            // feature_value	f(n) or su(n+1)
                            let bits = FEATURE_BITS[j] as usize;
                            let raw = if FEATURE_SIGNED[j] {
                                buf.get_su(bits + 1) as i16
                            } else {
                                buf.get_bits(bits) as i16
                            };
                            raw.clamp(-FEATURE_MAX[j], FEATURE_MAX[j])
                        } else {
                            0
                        };
                        features[i][j] = SegmentFeature { enabled, value };
                    }
                }
            }
        }

        Self {
            segmentation_enabled,
            segmentation_update_map,
            segmentation_temporal_update,
            segmentation_update_data,
            features,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Delta-Q and Delta-LF parameters
// ─────────────────────────────────────────────────────────────

/// Delta-Q parameters (block-level quantization adjustment).
///
/// AV1 spec Section 5.9.17 - delta_q_params().
#[derive(Debug, Clone)]
pub struct DeltaQParams {
    /// Whether block-level delta-Q is enabled.
    pub delta_q_present: bool,
    /// Resolution of delta-Q values (0 = finest).
    pub delta_q_res: u8,
}

impl DeltaQParams {
    pub fn decode(buf: &mut Buffer, base_q_idx: u8) -> Self {
        let delta_q_present = if base_q_idx > 0 {
            // delta_q_present_flag	f(1)
            buf.get_bit()
        } else {
            false
        };

        let delta_q_res = if delta_q_present {
            // delta_q_res	f(2)
            buf.get_bits(2) as u8
        } else {
            0
        };

        Self {
            delta_q_present,
            delta_q_res,
        }
    }
}

/// Delta loop-filter parameters (block-level loop filter adjustment).
///
/// AV1 spec Section 5.9.18 - delta_lf_params().
#[derive(Debug, Clone)]
pub struct DeltaLfParams {
    /// Whether block-level delta loop filter is enabled.
    pub delta_lf_present: bool,
    /// Resolution of delta-LF values.
    pub delta_lf_res: u8,
    /// Whether delta-LF applies independently to multiple filter components.
    pub delta_lf_multi: bool,
}

impl DeltaLfParams {
    pub fn decode(buf: &mut Buffer, delta_q_present: bool, allow_intrabc: bool) -> Self {
        let delta_lf_present = if delta_q_present && !allow_intrabc {
            // delta_lf_present_flag	f(1)
            buf.get_bit()
        } else {
            false
        };

        let (delta_lf_res, delta_lf_multi) = if delta_lf_present {
            // delta_lf_res	f(2)
            let res = buf.get_bits(2) as u8;
            // delta_lf_multi	f(1)
            let multi = buf.get_bit();
            (res, multi)
        } else {
            (0, false)
        };

        Self {
            delta_lf_present,
            delta_lf_res,
            delta_lf_multi,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Loop filter parameters
// ─────────────────────────────────────────────────────────────

/// Loop filter (deblocking) parameters.
///
/// AV1 spec Section 5.9.11 - loop_filter_params().
/// The deblocking filter reduces blocking artefacts at block boundaries.
#[derive(Debug, Clone)]
pub struct LoopFilterParams {
    /// Filter strength for 4 directions: [Y-vertical, Y-horizontal, U, V] (0–63).
    pub loop_filter_level: [u8; 4],
    /// Sharpness (0–7; higher values preserve more edges).
    pub loop_filter_sharpness: u8,
    /// Whether adaptive filter deltas are enabled (per reference-type and mode).
    pub loop_filter_delta_enabled: bool,
    /// Whether filter deltas are read from the bitstream (vs. inherited from the previous frame).
    pub loop_filter_delta_update: bool,
    /// Per-reference-type filter strength deltas (signed, −64–+63).
    pub loop_filter_ref_deltas: [i8; TOTAL_REFS_PER_FRAME as usize],
    /// Per-mode (intra/inter) filter strength deltas.
    pub loop_filter_mode_deltas: [i8; 2],
}

impl LoopFilterParams {
    pub fn decode(ctx: &ObuContext, buf: &mut Buffer, coded_lossless: bool, allow_intrabc: bool) -> Self {
        if coded_lossless || allow_intrabc {
            return Self::default_params();
        }

        // loop_filter_level[0]	f(6)  (Y vertical)
        // loop_filter_level[1]	f(6)  (Y horizontal)
        let lf_level_0 = buf.get_bits(6) as u8;
        let lf_level_1 = buf.get_bits(6) as u8;

        let (lf_level_2, lf_level_3) = if ctx.num_planes > 1 {
            // loop_filter_level[2]	f(6)  (U)
            // loop_filter_level[3]	f(6)  (V)
            (buf.get_bits(6) as u8, buf.get_bits(6) as u8)
        } else {
            (0, 0)
        };

        // loop_filter_sharpness	f(3)
        let loop_filter_sharpness = buf.get_bits(3) as u8;

        // loop_filter_delta_enabled	f(1)
        let loop_filter_delta_enabled = buf.get_bit();

        let mut loop_filter_ref_deltas = [1i8, 0, 0, 0, 0, -1, -1, -1]; // spec-defined defaults
        let mut loop_filter_mode_deltas = [0i8, 0];
        let mut loop_filter_delta_update = false;

        if loop_filter_delta_enabled {
            // loop_filter_delta_update	f(1)
            loop_filter_delta_update = buf.get_bit();
            if loop_filter_delta_update {
                for i in 0..TOTAL_REFS_PER_FRAME as usize {
                    // update_ref_delta	f(1)
                    if buf.get_bit() {
                        // loop_filter_ref_deltas[i]	su(7)
                        loop_filter_ref_deltas[i] = buf.get_su(7) as i8;
                    }
                }
                for i in 0..2 {
                    // update_mode_delta	f(1)
                    if buf.get_bit() {
                        // loop_filter_mode_deltas[i]	su(7)
                        loop_filter_mode_deltas[i] = buf.get_su(7) as i8;
                    }
                }
            }
        }

        Self {
            loop_filter_level: [lf_level_0, lf_level_1, lf_level_2, lf_level_3],
            loop_filter_sharpness,
            loop_filter_delta_enabled,
            loop_filter_delta_update,
            loop_filter_ref_deltas,
            loop_filter_mode_deltas,
        }
    }

    fn default_params() -> Self {
        Self {
            loop_filter_level: [0; 4],
            loop_filter_sharpness: 0,
            loop_filter_delta_enabled: false,
            loop_filter_delta_update: false,
            loop_filter_ref_deltas: [1, 0, 0, 0, 0, -1, -1, -1],
            loop_filter_mode_deltas: [0; 2],
        }
    }
}

// ─────────────────────────────────────────────────────────────
// CDEF parameters
// ─────────────────────────────────────────────────────────────

/// Constrained Directional Enhancement Filter (CDEF) parameters.
///
/// AV1 spec Section 5.9.19 - cdef_params().
/// CDEF is a post-processing filter that reduces ringing artefacts by
/// detecting edge directions and applying a directional filter.
#[derive(Debug, Clone)]
pub struct CdefParams {
    /// Damping factor (controls filter spread); actual value = damping_minus_3 + 3.
    pub cdef_damping: u8,
    /// log2 of the number of CDEF filter strengths (0–3 → 1–8 strengths).
    pub cdef_bits: u8,
    /// Y-plane primary strength for each CDEF level (controls directional filtering).
    pub cdef_y_pri_strength: Vec<u8>,
    /// Y-plane secondary strength for each CDEF level (controls diffusion).
    pub cdef_y_sec_strength: Vec<u8>,
    /// UV-plane primary strength for each CDEF level.
    pub cdef_uv_pri_strength: Vec<u8>,
    /// UV-plane secondary strength for each CDEF level.
    pub cdef_uv_sec_strength: Vec<u8>,
}

impl CdefParams {
    pub fn decode(ctx: &ObuContext, buf: &mut Buffer, coded_lossless: bool, allow_intrabc: bool) -> Self {
        let seq = ctx.sequence_header.as_ref().unwrap();

        if coded_lossless || allow_intrabc || !seq.enable_cdef {
            return Self::default_params();
        }

        // cdef_damping_minus_3	f(2)
        let cdef_damping = buf.get_bits(2) as u8 + 3;

        // cdef_bits	f(2)  — there are 1 << cdef_bits CDEF strengths
        let cdef_bits = buf.get_bits(2) as u8;
        let num_cdef_strengths = 1usize << cdef_bits;

        let mut cdef_y_pri_strength = Vec::with_capacity(num_cdef_strengths);
        let mut cdef_y_sec_strength = Vec::with_capacity(num_cdef_strengths);
        let mut cdef_uv_pri_strength = Vec::with_capacity(num_cdef_strengths);
        let mut cdef_uv_sec_strength = Vec::with_capacity(num_cdef_strengths);

        for _ in 0..num_cdef_strengths {
            // cdef_y_pri_strength	f(4)  (0 = disabled)
            cdef_y_pri_strength.push(buf.get_bits(4) as u8);
            // cdef_y_sec_strength	f(2)  (values 0,1,2,4; coded 3 maps to 4)
            let sec = buf.get_bits(2) as u8;
            cdef_y_sec_strength.push(if sec == 3 { 4 } else { sec });

            if ctx.num_planes > 1 {
                cdef_uv_pri_strength.push(buf.get_bits(4) as u8);
                let sec = buf.get_bits(2) as u8;
                cdef_uv_sec_strength.push(if sec == 3 { 4 } else { sec });
            }
        }

        Self {
            cdef_damping,
            cdef_bits,
            cdef_y_pri_strength,
            cdef_y_sec_strength,
            cdef_uv_pri_strength,
            cdef_uv_sec_strength,
        }
    }

    fn default_params() -> Self {
        Self {
            cdef_damping: 3,
            cdef_bits: 0,
            cdef_y_pri_strength: vec![0],
            cdef_y_sec_strength: vec![0],
            cdef_uv_pri_strength: vec![],
            cdef_uv_sec_strength: vec![],
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Loop restoration parameters
// ─────────────────────────────────────────────────────────────

/// Loop restoration filter type applied per plane.
///
/// AV1 spec Section 6.10.15 - FrameRestorationType.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestorationType {
    /// No restoration (pass-through).
    None,
    /// Wiener filter (linear FIR filter; low complexity, good quality).
    Wiener,
    /// Self-Guided Projection (non-linear filter; high quality).
    SgrProj,
    /// Switchable — decoder selects per restoration unit.
    Switchable,
}

impl From<u8> for RestorationType {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::None,
            1 => Self::Wiener,
            2 => Self::SgrProj,
            3 => Self::Switchable,
            _ => Self::None,
        }
    }
}

/// Loop restoration parameters.
///
/// AV1 spec Section 5.9.20 - lr_params().
/// Loop restoration is applied after deblocking and CDEF to recover high-
/// frequency detail.
#[derive(Debug, Clone)]
pub struct LrParams {
    /// Restoration type for each plane (up to 3 planes).
    pub lr_type: [RestorationType; 3],
    /// Restoration unit size shift (determines the block size for restoration).
    pub lr_unit_shift: u8,
    /// Whether chroma planes use a smaller restoration unit than luma.
    pub lr_uv_shift: bool,
}

impl LrParams {
    pub fn decode(ctx: &ObuContext, buf: &mut Buffer, all_lossless: bool, allow_intrabc: bool) -> Self {
        let seq = ctx.sequence_header.as_ref().unwrap();

        if all_lossless || allow_intrabc || !seq.enable_restoration {
            return Self {
                lr_type: [RestorationType::None; 3],
                lr_unit_shift: 0,
                lr_uv_shift: false,
            };
        }

        let mut lr_type = [RestorationType::None; 3];
        let mut uses_lr = false;
        let mut uses_chroma_lr = false;

        for i in 0..ctx.num_planes as usize {
            // lr_type	f(2)
            lr_type[i] = RestorationType::from(buf.get_bits(2) as u8);
            if lr_type[i] != RestorationType::None {
                uses_lr = true;
                if i > 0 {
                    uses_chroma_lr = true;
                }
            }
        }

        let mut lr_unit_shift = 0u8;
        let mut lr_uv_shift = false;

        if uses_lr {
            if ctx.superres_denom != SUPERRES_NUM {
                // With super-resolution: lr_unit_shift starts at 2 and can extend.
                // lr_unit_shift	f(1)
                lr_unit_shift = buf.get_bit() as u8;
                if lr_unit_shift != 0 {
                    // lr_unit_extra_shift	f(1)
                    let extra = buf.get_bit() as u8;
                    lr_unit_shift += extra;
                }
                lr_unit_shift += 2;
            } else {
                // Without super-resolution: lr_unit_shift starts at 1.
                // lr_unit_shift	f(1)
                lr_unit_shift = buf.get_bit() as u8 + 1;
            }

            if ctx.num_planes > 1 && uses_chroma_lr {
                // lr_uv_shift	f(1)
                lr_uv_shift = buf.get_bit();
            }
        }

        Self {
            lr_type,
            lr_unit_shift,
            lr_uv_shift,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Global motion parameters
// ─────────────────────────────────────────────────────────────

/// Global motion parameters for all reference frames.
///
/// AV1 spec Section 5.9.24 - global_motion_params().
/// Global motion describes camera motion or large-area background motion
/// using one of four models: identity, translation, rotation+zoom, affine.
#[derive(Debug, Clone)]
pub struct GlobalMotionParams {
    /// Motion model type for each of the REFS_PER_FRAME (7) reference frames.
    pub gm_type: [GmType; REFS_PER_FRAME as usize],
    /// Motion model parameters for each reference frame.
    /// params[i] = [alpha, beta, gamma, delta, tx, ty] at WARPEDMODEL_PREC_BITS precision.
    pub gm_params: [[i32; 6]; REFS_PER_FRAME as usize],
}

/// Decode a sub-exponential coded unsigned integer.
///
/// AV1 spec Section 4.10.7 - decode_subexp().
fn decode_subexp(buf: &mut Buffer, num_syms: u32, k: u32) -> u32 {
    let mut i = 0u32;
    let mut mk = 0u32;
    loop {
        let b2 = if i > 0 { k + i - 1 } else { k };
        let a = 1u32 << b2;
        if num_syms <= mk + 3 * a {
            // Remaining values use fixed-length coding.
            let bits = 32u32 - (num_syms - mk).leading_zeros();
            return buf.get_bits(bits as usize) + mk;
        } else {
            // flag	f(1): 0 = use b2-bit code, 1 = continue to next tier
            let flag = buf.get_bit();
            if flag {
                i += 1;
                mk += a;
            } else {
                return buf.get_bits(b2 as usize) + mk;
            }
        }
    }
}

/// Inverse re-centering around reference value r.
fn inverse_recenter(r: i32, v: u32) -> i32 {
    if v > 2 * r.unsigned_abs() {
        v as i32
    } else if v & 1 != 0 {
        r - ((v as i32 + 1) >> 1)
    } else {
        r + (v as i32 >> 1)
    }
}

/// Decode a sub-exponential signed integer relative to reference r.
fn decode_signed_subexp_with_ref(buf: &mut Buffer, low: i32, high: i32, r: i32) -> i32 {
    let x = decode_subexp(
        buf,
        (high - low) as u32,
        SGRPROJ_PRJ_SUBEXP_K as u32,
    );
    let v = inverse_recenter(r - low, x);
    v + low
}

/// Read a single global motion parameter.
///
/// AV1 spec Section 5.9.25 - read_global_param().
fn read_global_param(
    buf: &mut Buffer,
    gm_type: GmType,
    prev_param: i32,
    idx: usize,
    allow_high_precision_mv: bool,
) -> i32 {
    use crate::constants::{
        GM_ABS_ALPHA_BITS, GM_ABS_TRANS_BITS, GM_ABS_TRANS_ONLY_BITS, GM_ALPHA_PREC_BITS,
        GM_TRANS_ONLY_PREC_BITS, GM_TRANS_PREC_BITS, WARPEDMODEL_PREC_BITS,
    };

    // Select coding precision based on parameter index and motion model type.
    let (abs_bits, prec_bits) = if idx < 2 {
        if gm_type == GmType::Translation {
            let ab = GM_ABS_TRANS_ONLY_BITS - !allow_high_precision_mv as u8;
            let pb = GM_TRANS_ONLY_PREC_BITS - !allow_high_precision_mv as u8;
            (ab, pb)
        } else {
            (GM_ABS_TRANS_BITS, GM_TRANS_PREC_BITS)
        }
    } else {
        (GM_ABS_ALPHA_BITS, GM_ALPHA_PREC_BITS)
    };

    let prec_diff = WARPEDMODEL_PREC_BITS - prec_bits;
    // Diagonal elements (idx % 3 == 2) have an additional bias of 1 << WARPEDMODEL_PREC_BITS.
    let round = if idx % 3 == 2 {
        1i32 << WARPEDMODEL_PREC_BITS
    } else {
        0
    };
    let sub = if idx % 3 == 2 {
        1i32 << prec_bits
    } else {
        0
    };

    let mx = 1i32 << abs_bits;
    let r = (prev_param >> prec_diff as i32) - sub;

    (decode_signed_subexp_with_ref(buf, -mx, mx + 1, r) << prec_diff as i32) + round
}

impl GlobalMotionParams {
    pub fn decode(
        buf: &mut Buffer,
        frame_is_intra: bool,
        allow_high_precision_mv: bool,
        prev_gm_params: &[[i32; 6]; REFS_PER_FRAME as usize],
    ) -> Self {
        let mut gm_type = [GmType::Identity; REFS_PER_FRAME as usize];
        let mut gm_params = [[0i32; 6]; REFS_PER_FRAME as usize];

        // Intra frames have no global motion parameters.
        if frame_is_intra {
            return Self { gm_type, gm_params };
        }

        for ref_idx in 0..REFS_PER_FRAME as usize {
            // is_global	f(1)
            let is_global = buf.get_bit();

            if is_global {
                // is_rot_zoom	f(1)
                let is_rot_zoom = buf.get_bit();
                if is_rot_zoom {
                    gm_type[ref_idx] = GmType::RotZoom;
                } else {
                    // is_translation	f(1)
                    let is_translation = buf.get_bit();
                    gm_type[ref_idx] = if is_translation {
                        GmType::Translation
                    } else {
                        GmType::Affine
                    };
                }
            } else {
                gm_type[ref_idx] = GmType::Identity;
            }

            if gm_type[ref_idx] >= GmType::RotZoom {
                // Parameters 2 and 3 (non-translational components).
                gm_params[ref_idx][2] = read_global_param(
                    buf,
                    gm_type[ref_idx],
                    prev_gm_params[ref_idx][2],
                    2,
                    allow_high_precision_mv,
                );
                gm_params[ref_idx][3] = read_global_param(
                    buf,
                    gm_type[ref_idx],
                    prev_gm_params[ref_idx][3],
                    3,
                    allow_high_precision_mv,
                );

                if gm_type[ref_idx] == GmType::Affine {
                    // Affine model requires parameters 4 and 5 as well.
                    gm_params[ref_idx][4] = read_global_param(
                        buf,
                        gm_type[ref_idx],
                        prev_gm_params[ref_idx][4],
                        4,
                        allow_high_precision_mv,
                    );
                    gm_params[ref_idx][5] = read_global_param(
                        buf,
                        gm_type[ref_idx],
                        prev_gm_params[ref_idx][5],
                        5,
                        allow_high_precision_mv,
                    );
                } else {
                    // RotZoom: parameters 4, 5 are derived from 2, 3 (symmetry constraint).
                    use crate::constants::WARPEDMODEL_PREC_BITS;
                    gm_params[ref_idx][4] = -gm_params[ref_idx][3];
                    gm_params[ref_idx][5] = gm_params[ref_idx][2];
                    let _ = WARPEDMODEL_PREC_BITS;
                }
            }

            if gm_type[ref_idx] >= GmType::Translation {
                // Translation components (parameters 0 and 1).
                gm_params[ref_idx][0] = read_global_param(
                    buf,
                    gm_type[ref_idx],
                    prev_gm_params[ref_idx][0],
                    0,
                    allow_high_precision_mv,
                );
                gm_params[ref_idx][1] = read_global_param(
                    buf,
                    gm_type[ref_idx],
                    prev_gm_params[ref_idx][1],
                    1,
                    allow_high_precision_mv,
                );
            }
        }

        Self { gm_type, gm_params }
    }
}

// Implement ordering on GmType so we can use >= comparisons.
impl PartialOrd for GmType {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GmType {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let rank = |t: &GmType| match t {
            GmType::Identity => 0,
            GmType::Translation => 1,
            GmType::RotZoom => 2,
            GmType::Affine => 3,
        };
        rank(self).cmp(&rank(other))
    }
}

// ─────────────────────────────────────────────────────────────
// Film grain parameters (simplified: parses enough to stay in sync)
// ─────────────────────────────────────────────────────────────

/// Film grain synthesis parameters.
///
/// AV1 spec Section 5.9.30 - film_grain_params().
/// Film grain is a post-processing effect that adds simulated analogue
/// film grain to the decoded picture.
#[derive(Debug, Clone)]
pub struct FilmGrainParams {
    /// Whether film grain is applied to this frame.
    pub apply_grain: bool,
    /// Pseudo-random seed for the grain pattern.
    pub grain_seed: u16,
    /// Number of luma scaling points (AR model order indicator).
    pub num_y_points: u8,
}

impl FilmGrainParams {
    pub fn decode(
        buf: &mut Buffer,
        film_grain_params_present: bool,
        show_frame: bool,
        showable_frame: bool,
    ) -> Option<Self> {
        if !film_grain_params_present || (!show_frame && !showable_frame) {
            return None;
        }

        // apply_grain	f(1)
        let apply_grain = buf.get_bit();
        if !apply_grain {
            return Some(Self {
                apply_grain: false,
                grain_seed: 0,
                num_y_points: 0,
            });
        }

        // grain_seed	f(16)
        let grain_seed = buf.get_bits(16) as u16;

        // update_grain	f(1)
        let update_grain = buf.get_bit();

        if !update_grain {
            // film_grain_params_ref_idx	f(3)
            buf.get_bits(3);
            return Some(Self {
                apply_grain,
                grain_seed,
                num_y_points: 0,
            });
        }

        // num_y_points	f(4)  (0 = luma grain disabled)
        let num_y_points = buf.get_bits(4) as u8;

        for _ in 0..num_y_points {
            buf.get_bits(8); // point_y_value[i]	f(8)
            buf.get_bits(8); // point_y_scaling[i]	f(8)
        }

        // chroma_scaling_from_luma	f(1)
        let chroma_scaling_from_luma = buf.get_bit();

        let (num_cb_points, num_cr_points) = if chroma_scaling_from_luma {
            (0u8, 0u8)
        } else {
            // num_cb_points	f(4)
            let cb = buf.get_bits(4) as u8;
            for _ in 0..cb {
                buf.get_bits(8);
                buf.get_bits(8);
            }
            // num_cr_points	f(4)
            let cr = buf.get_bits(4) as u8;
            for _ in 0..cr {
                buf.get_bits(8);
                buf.get_bits(8);
            }
            (cb, cr)
        };

        // grain_scaling_minus_8	f(2)
        buf.get_bits(2);
        // ar_coeff_lag	f(2)  (AR model order)
        let ar_coeff_lag = buf.get_bits(2) as usize;

        // num AR coefficients for luma = (2*lag+1)^2 - 1
        let num_pos_luma = 2 * ar_coeff_lag * (ar_coeff_lag + 1);
        let num_pos_chroma = if num_y_points > 0 {
            num_pos_luma + 1
        } else {
            num_pos_luma
        };

        for _ in 0..num_pos_luma {
            buf.get_bits(8); // ar_coeffs_y_plus_128[i]
        }
        if chroma_scaling_from_luma || num_cb_points > 0 {
            for _ in 0..num_pos_chroma {
                buf.get_bits(8); // ar_coeffs_cb_plus_128[i]
            }
        }
        if chroma_scaling_from_luma || num_cr_points > 0 {
            for _ in 0..num_pos_chroma {
                buf.get_bits(8); // ar_coeffs_cr_plus_128[i]
            }
        }

        // ar_coeff_shift_minus_6	f(2)
        buf.get_bits(2);
        // grain_scale_shift	f(2)
        buf.get_bits(2);

        if num_cb_points > 0 {
            buf.get_bits(8); // cb_mult
            buf.get_bits(8); // cb_luma_mult
            buf.get_bits(9); // cb_offset
        }
        if num_cr_points > 0 {
            buf.get_bits(8); // cr_mult
            buf.get_bits(8); // cr_luma_mult
            buf.get_bits(9); // cr_offset
        }

        // overlap_flag	f(1)
        buf.get_bit();
        // clip_to_restricted_range	f(1)
        buf.get_bit();

        Some(Self {
            apply_grain,
            grain_seed,
            num_y_points,
        })
    }
}

// ─────────────────────────────────────────────────────────────
// Interpolation filter
// ─────────────────────────────────────────────────────────────

/// Read the interpolation filter field.
///
/// AV1 spec Section 5.9.12 - read_interpolation_filter().
#[inline]
pub fn read_interpolation_filter(buf: &mut Buffer) -> Result<InterpolationFilter, ObuError> {
    // is_filter_switchable	f(1)
    let is_filter_switchable = buf.get_bit();
    Ok(if is_filter_switchable {
        InterpolationFilter::Switchable
    } else {
        // interpolation_filter	f(2)
        InterpolationFilter::try_from(buf.get_bits(2) as u8)?
    })
}

// ─────────────────────────────────────────────────────────────
// Frame header
// ─────────────────────────────────────────────────────────────

/// Complete AV1 frame header.
///
/// Corresponds to AV1 spec Section 5.9 - uncompressed_header().
/// Contains the frame type, dimensions, and all coding parameters.
#[derive(Debug, Clone)]
pub struct FrameHeader {
    // ── Frame basics ─────────────────────────────────────────
    /// Whether this OBU re-displays a previously coded reference frame.
    pub show_existing_frame: bool,
    /// Frame type (key / inter / intra-only / switch).
    pub frame_type: FrameType,
    /// Whether this frame should be output for display.
    pub show_frame: bool,
    /// Whether this frame can be re-displayed via show_existing_frame.
    pub showable_frame: bool,
    /// Whether error-resilient mode is active (affects CDF and reference semantics).
    pub error_resilient_mode: bool,

    // ── Coding tool flags ────────────────────────────────────
    /// Whether CDF (context distribution function) updates are disabled.
    pub disable_cdf_update: bool,
    /// Whether screen content coding tools (palette, IntraBC) are allowed.
    pub allow_screen_content_tools: bool,
    /// Whether motion vectors are forced to integer precision.
    pub force_integer_mv: bool,

    // ── Frame identity ───────────────────────────────────────
    /// Current frame ID (used for reference frame validity checks).
    pub current_frame_id: u32,

    // ── Frame dimensions ─────────────────────────────────────
    /// Whether the sequence header's max frame size is overridden.
    pub frame_size_override: bool,
    /// Coded frame width in luma samples.
    pub frame_width: u16,
    /// Coded frame height in luma samples.
    pub frame_height: u16,
    /// Frame width after super-resolution upscaling.
    pub upscaled_width: u16,
    /// Suggested display width.
    pub render_width: u16,
    /// Suggested display height.
    pub render_height: u16,

    // ── Frame ordering ───────────────────────────────────────
    /// Display order hint (used for B-frame reordering).
    pub order_hint: u32,
    /// Primary reference frame index (initialises the entropy coder).
    pub primary_ref_frame: u8,
    /// Bitmask of reference buffer slots refreshed by this frame (8 bits).
    pub refresh_frame_flags: u32,

    // ── Reference frame signalling (inter frames only) ───────
    /// Indices into the reference buffer for each of REFS_PER_FRAME (7) ref types.
    pub ref_frame_idx: [u8; REFS_PER_FRAME as usize],
    /// Whether 1/8-pixel precision motion vectors are allowed.
    pub allow_high_precision_mv: bool,
    /// Interpolation filter type (None for intra frames).
    pub interpolation_filter: Option<InterpolationFilter>,
    /// Whether per-block motion mode switching is allowed.
    pub is_motion_mode_switchable: bool,
    /// Whether reference frame motion vectors (MFMV) are used.
    pub use_ref_frame_mvs: bool,

    // ── IntraBC (screen content) ─────────────────────────────
    /// Whether intra block copy (IntraBC) is allowed.
    pub allow_intrabc: bool,

    // ── CDF update control ───────────────────────────────────
    /// Whether end-of-frame CDF updates are disabled.
    pub disable_frame_end_update_cdf: bool,

    // ── Structural parameters ────────────────────────────────
    /// Tile grid layout.
    pub tile_info: TileInfo,
    /// Quantization parameters.
    pub quantization_params: QuantizationParams,
    /// Segmentation parameters.
    pub segmentation_params: SegmentationParams,
    /// Block-level delta-Q parameters.
    pub delta_q_params: DeltaQParams,
    /// Block-level delta loop-filter parameters.
    pub delta_lf_params: DeltaLfParams,
    /// Loop (deblocking) filter parameters.
    pub loop_filter_params: LoopFilterParams,
    /// CDEF parameters.
    pub cdef_params: CdefParams,
    /// Loop restoration parameters.
    pub lr_params: LrParams,

    // ── Transform and reference modes ────────────────────────
    /// Transform size selection mode.
    pub tx_mode: TxMode,
    /// Whether compound prediction reference pairs are selected from the bitstream.
    pub reference_select: bool,
    /// Whether skip mode is signalled (zero-residual inter prediction).
    pub skip_mode_present: bool,

    // ── Motion parameters ────────────────────────────────────
    /// Whether local affine (warped) motion is allowed.
    pub allow_warped_motion: bool,
    /// Whether a reduced transform set is used.
    pub reduced_tx_set: bool,
    /// Global motion parameters for each reference frame.
    pub global_motion_params: GlobalMotionParams,

    // ── Optional fields ──────────────────────────────────────
    /// Frame presentation timestamp (present only when decoder model and non-equal
    /// picture interval timing are signalled).
    pub temporal_point_info: Option<TemporalPointInfo>,
    /// Decoder buffer removal times, one per operating point.
    pub buffer_removal_times: Vec<u32>,
    /// Film grain synthesis parameters (if enabled).
    pub film_grain_params: Option<FilmGrainParams>,
}

impl FrameHeader {
    /// Decode the complete frame header (uncompressed_header).
    ///
    /// AV1 spec Section 5.9 - uncompressed_header() and
    /// Section 5.8 - frame_header_obu().
    pub fn decode(ctx: &mut ObuContext, buf: &mut Buffer) -> Result<Self, ObuError> {
        // frame_header_copy: if seen_frame_header is set the spec says the
        // header is a copy of the previous one. For this analysis tool we
        // simply continue parsing (a full decoder would cache and return the copy).
        ctx.seen_frame_header = true;

        let sequence_header = ctx
            .sequence_header
            .as_ref()
            .cloned()
            .ok_or(ObuError::NotFoundSequenceHeader)?;

        // idLen: total bits for a frame ID field
        let id_len = if let Some(ref f) = sequence_header.frame_id_numbers_present {
            f.additional_frame_id_length as usize + f.delta_frame_id_length as usize + 3
        } else {
            0
        };

        let all_frames = (1u32 << NUM_REF_FRAMES) - 1;

        // ── Field initialisation ───────────────────────────────
        let mut show_existing_frame = false;
        let mut frame_type = FrameType::KeyFrame;
        let mut show_frame = true;
        let mut showable_frame = false;
        let mut error_resilient_mode = false;
        let mut refresh_frame_flags = 0u32;
        let mut temporal_point_info = None;
        let mut ref_frame_idx = [0u8; REFS_PER_FRAME as usize];

        if sequence_header.reduced_still_picture_header {
            // Reduced still-picture header: fixed key frame, intra-only.
            ctx.frame_is_intra = true;
        } else {
            // show_existing_frame	f(1)
            show_existing_frame = buf.get_bit();

            if show_existing_frame {
                // ── Re-display an already coded frame ─────────────
                // frame_to_show_map_idx	f(3)
                let frame_to_show_map_idx = buf.get_bits(3) as u8;

                if let Some(ref dmi) = sequence_header.decoder_model_info {
                    if !sequence_header
                        .timing_info
                        .map(|v| v.equal_picture_interval.is_some())
                        .unwrap_or(false)
                    {
                        temporal_point_info = Some(TemporalPointInfo::decode(
                            buf,
                            dmi.frame_presentation_time_length as usize,
                        ));
                    }
                }

                if sequence_header.frame_id_numbers_present.is_some() {
                    // display_frame_id	f(idLen)
                    buf.get_bits(id_len);
                }

                frame_type = ctx
                    .ref_frame_type
                    .get(frame_to_show_map_idx as usize)
                    .copied()
                    .ok_or(ObuError::Unknown(ObuUnknownError::FrameTypeRefIndex))?;

                if frame_type == FrameType::KeyFrame {
                    refresh_frame_flags = all_frames;
                }

                // TODO: load_grain_params(frame_to_show_map_idx)

                ctx.seen_frame_header = false;
                return Ok(Self::show_existing(
                    frame_to_show_map_idx,
                    frame_type,
                    refresh_frame_flags,
                    temporal_point_info,
                ));
            }

            // ── Normal frame header ────────────────────────────
            // frame_type	f(2)
            frame_type = FrameType::try_from(buf.get_bits(2) as u8)?;
            ctx.frame_is_intra =
                frame_type == FrameType::IntraOnlyFrame || frame_type == FrameType::KeyFrame;

            // show_frame	f(1)
            show_frame = buf.get_bit();

            if show_frame {
                if let Some(ref dmi) = sequence_header.decoder_model_info {
                    if !sequence_header
                        .timing_info
                        .map(|v| v.equal_picture_interval.is_some())
                        .unwrap_or(false)
                    {
                        temporal_point_info = Some(TemporalPointInfo::decode(
                            buf,
                            dmi.frame_presentation_time_length as usize,
                        ));
                    }
                }
                showable_frame = frame_type != FrameType::KeyFrame;
            } else {
                // showable_frame	f(1)
                showable_frame = buf.get_bit();
            }

            // SwitchFrame and key+show frames are always error-resilient.
            error_resilient_mode = if frame_type == FrameType::SwitchFrame
                || (frame_type == FrameType::KeyFrame && show_frame)
            {
                true
            } else {
                // error_resilient_mode	f(1)
                buf.get_bit()
            };
        }

        // disable_cdf_update	f(1)
        let disable_cdf_update = buf.get_bit();

        let allow_screen_content_tools =
            if sequence_header.seq_force_screen_content_tools == SELECT_SCREEN_CONTENT_TOOLS {
                // allow_screen_content_tools	f(1)
                buf.get_bit()
            } else {
                sequence_header.seq_force_screen_content_tools != 0
            };

        // Intra frames always use integer MV precision.
        let mut force_integer_mv = if allow_screen_content_tools && !ctx.frame_is_intra {
            if sequence_header.seq_force_integer_mv == SELECT_INTEGER_MV {
                // force_integer_mv	f(1)
                buf.get_bit()
            } else {
                sequence_header.seq_force_integer_mv != 0
            }
        } else {
            false
        };
        if ctx.frame_is_intra {
            force_integer_mv = true;
        }

        let current_frame_id = if sequence_header.frame_id_numbers_present.is_some() {
            // current_frame_id	f(idLen)
            let fid = buf.get_bits(id_len);
            // TODO: mark_ref_frames(idLen)
            fid
        } else {
            0
        };

        let frame_size_override = if frame_type == FrameType::SwitchFrame {
            true
        } else if sequence_header.reduced_still_picture_header {
            false
        } else {
            // frame_size_override_flag	f(1)
            buf.get_bit()
        };

        // order_hint	f(OrderHintBits)
        let order_hint = buf.get_bits(ctx.order_hint_bits);
        ctx.order_hint = order_hint;

        let primary_ref_frame = if ctx.frame_is_intra || error_resilient_mode {
            PRIMARY_REF_NONE
        } else {
            // primary_ref_frame	f(3)
            buf.get_bits(3) as u8
        };

        // Decoder buffer removal times (one per applicable operating point).
        let mut buffer_removal_times = Vec::new();
        if let Some(ref dmi) = sequence_header.decoder_model_info {
            // buffer_removal_time_present_flag	f(1)
            let present = buf.get_bit();
            if present {
                for op in &sequence_header.operating_points {
                    if op.operating_parameters_info.is_some() {
                        if let Some(ext) = ctx.obu_header_extension {
                            let op_idc = op.idc;
                            let in_temporal = ((op_idc >> ext.temporal_id) & 1) != 0;
                            let in_spatial = ((op_idc >> (ext.spatial_id + 8)) & 1) != 0;
                            if op_idc == 0 || (in_temporal && in_spatial) {
                                // buffer_removal_time	f(n)
                                buffer_removal_times.push(
                                    buf.get_bits(dmi.buffer_removal_time_length as usize),
                                );
                            }
                        }
                    }
                }
            }
        }

        // Key frames and switch frames refresh all reference buffers.
        refresh_frame_flags = if frame_type == FrameType::SwitchFrame
            || (frame_type == FrameType::KeyFrame && show_frame)
        {
            all_frames
        } else if frame_type == FrameType::IntraOnlyFrame && sequence_header.reduced_still_picture_header {
            all_frames
        } else {
            // refresh_frame_flags	f(8)
            buf.get_bits(8)
        };

        // In error-resilient mode, ref_order_hints are read to validate refs.
        if (!ctx.frame_is_intra || refresh_frame_flags != all_frames)
            && error_resilient_mode
            && sequence_header.enable_order_hint
        {
            for i in 0..NUM_REF_FRAMES as usize {
                // ref_order_hint	f(OrderHintBits)
                let hint = buf.get_bits(ctx.order_hint_bits);
                if ctx.ref_order_hint[i] != hint {
                    ctx.ref_frame_marking[i] = false;
                }
                ctx.ref_order_hint[i] = hint;
            }
        }

        let mut allow_intrabc = false;

        if ctx.frame_is_intra {
            frame_size(ctx, frame_size_override, buf);
            render_size(ctx, buf);

            if allow_screen_content_tools && ctx.upscaled_width == ctx.frame_width {
                // allow_intrabc	f(1)  (only when superres is not active)
                allow_intrabc = buf.get_bit();
            }
        } else {
            // ── Inter frame: reference frame signalling ───────
            let mut frame_refs_short_signaling = false;
            if sequence_header.enable_order_hint {
                // frame_refs_short_signaling	f(1)
                frame_refs_short_signaling = buf.get_bit();
                if frame_refs_short_signaling {
                    // last_frame_idx	f(3)
                    let _last_frame_idx = buf.get_bits(3) as u8;
                    // gold_frame_idx	f(3)
                    let _gold_frame_idx = buf.get_bits(3) as u8;
                    // TODO: set_frame_refs() — infer remaining reference indices
                    // from order hints. Omitted here as this tool does not
                    // perform full reference management.
                }
            }

            for i in 0..REFS_PER_FRAME as usize {
                if !frame_refs_short_signaling {
                    // ref_frame_idx[i]	f(3)
                    ref_frame_idx[i] = buf.get_bits(3) as u8;
                }

                if let Some(ref fidp) = sequence_header.frame_id_numbers_present {
                    let n = fidp.delta_frame_id_length as usize;
                    // delta_frame_id_minus_1	f(n)
                    let delta_frame_id = buf.get_bits(n) + 1;
                    ctx.delta_frame_id = delta_frame_id;
                    // TODO: validate expectedFrameId[i]
                }
            }
            ctx.ref_frame_idx = ref_frame_idx;

            if frame_size_override && !error_resilient_mode {
                frame_size_with_refs(ctx, frame_size_override, &ref_frame_idx, buf);
            } else {
                frame_size(ctx, frame_size_override, buf);
                render_size(ctx, buf);
            }

            // allow_high_precision_mv	f(1)
            let allow_high_precision_mv = if force_integer_mv {
                false
            } else {
                buf.get_bit()
            };

            let interpolation_filter = read_interpolation_filter(buf)?;

            // is_motion_mode_switchable	f(1)
            let is_motion_mode_switchable = buf.get_bit();

            // use_ref_frame_mvs	f(1)
            let use_ref_frame_mvs = if error_resilient_mode
                || !sequence_header.enable_ref_frame_mvs
            {
                false
            } else {
                buf.get_bit()
            };

            let disable_frame_end_update_cdf =
                if sequence_header.reduced_still_picture_header || disable_cdf_update {
                    true
                } else {
                    // disable_frame_end_update_cdf	f(1)
                    buf.get_bit()
                };

            // TODO: init_non_coeff_cdfs / load_cdfs based on primary_ref_frame

            let tile_info = TileInfo::decode(ctx, buf);
            let quantization_params = QuantizationParams::decode(ctx, buf);
            let segmentation_params = SegmentationParams::decode(buf, ctx.frame_is_intra, primary_ref_frame);
            let delta_q_params = DeltaQParams::decode(buf, quantization_params.base_q_idx);
            let delta_lf_params = DeltaLfParams::decode(buf, delta_q_params.delta_q_present, allow_intrabc);

            let coded_lossless = quantization_params.is_lossless();
            let all_lossless = coded_lossless && ctx.frame_width == ctx.upscaled_width;

            let loop_filter_params = LoopFilterParams::decode(ctx, buf, coded_lossless, allow_intrabc);
            let cdef_params = CdefParams::decode(ctx, buf, coded_lossless, allow_intrabc);
            let lr_params = LrParams::decode(ctx, buf, all_lossless, allow_intrabc);

            // read_tx_mode
            let tx_mode = if coded_lossless {
                TxMode::Only4x4
            } else {
                // tx_mode_select	f(1)
                if buf.get_bit() {
                    TxMode::Select
                } else {
                    TxMode::Largest
                }
            };

            // frame_reference_mode: compound prediction
            let reference_select = if ctx.frame_is_intra {
                false
            } else {
                // reference_select	f(1)
                buf.get_bit()
            };

            let skip_mode_present = decode_skip_mode_params(
                buf,
                &sequence_header,
                ctx,
                &ref_frame_idx,
                frame_type,
                reference_select,
            );

            let allow_warped_motion = if ctx.frame_is_intra
                || error_resilient_mode
                || !sequence_header.enable_warped_motion
            {
                false
            } else {
                // allow_warped_motion	f(1)
                buf.get_bit()
            };

            // reduced_tx_set	f(1)
            let reduced_tx_set = buf.get_bit();

            let prev_gm = ctx.prev_gm_params;
            let global_motion_params = GlobalMotionParams::decode(
                buf,
                ctx.frame_is_intra,
                allow_high_precision_mv,
                &prev_gm,
            );

            let film_grain_params = FilmGrainParams::decode(
                buf,
                sequence_header.film_grain_params_present,
                show_frame,
                showable_frame,
            );

            update_ref_frame_context(ctx, refresh_frame_flags, frame_type);

            return Ok(FrameHeader {
                show_existing_frame,
                frame_type,
                show_frame,
                showable_frame,
                error_resilient_mode,
                disable_cdf_update,
                allow_screen_content_tools,
                force_integer_mv,
                current_frame_id,
                frame_size_override,
                frame_width: ctx.frame_width,
                frame_height: ctx.frame_height,
                upscaled_width: ctx.upscaled_width,
                render_width: ctx.render_width,
                render_height: ctx.render_height,
                order_hint,
                primary_ref_frame,
                refresh_frame_flags,
                ref_frame_idx,
                allow_high_precision_mv,
                interpolation_filter: Some(interpolation_filter),
                is_motion_mode_switchable,
                use_ref_frame_mvs,
                allow_intrabc,
                disable_frame_end_update_cdf,
                tile_info,
                quantization_params,
                segmentation_params,
                delta_q_params,
                delta_lf_params,
                loop_filter_params,
                cdef_params,
                lr_params,
                tx_mode,
                reference_select,
                skip_mode_present,
                allow_warped_motion,
                reduced_tx_set,
                global_motion_params,
                temporal_point_info,
                buffer_removal_times,
                film_grain_params,
            });
        }

        // ── Intra frame continuation ───────────────────────────
        let disable_frame_end_update_cdf =
            if sequence_header.reduced_still_picture_header || disable_cdf_update {
                true
            } else {
                // disable_frame_end_update_cdf	f(1)
                buf.get_bit()
            };

        // TODO: init_non_coeff_cdfs / load_cdfs based on primary_ref_frame

        let tile_info = TileInfo::decode(ctx, buf);
        let quantization_params = QuantizationParams::decode(ctx, buf);
        let segmentation_params = SegmentationParams::decode(buf, ctx.frame_is_intra, primary_ref_frame);
        let delta_q_params = DeltaQParams::decode(buf, quantization_params.base_q_idx);
        let delta_lf_params = DeltaLfParams::decode(buf, delta_q_params.delta_q_present, allow_intrabc);

        let coded_lossless = quantization_params.is_lossless();
        let all_lossless = coded_lossless && ctx.frame_width == ctx.upscaled_width;

        let loop_filter_params = LoopFilterParams::decode(ctx, buf, coded_lossless, allow_intrabc);
        let cdef_params = CdefParams::decode(ctx, buf, coded_lossless, allow_intrabc);
        let lr_params = LrParams::decode(ctx, buf, all_lossless, allow_intrabc);

        let tx_mode = if coded_lossless {
            TxMode::Only4x4
        } else {
            if buf.get_bit() {
                TxMode::Select
            } else {
                TxMode::Largest
            }
        };

        let reference_select = false; // intra frames have no compound prediction
        let skip_mode_present = false; // intra frames have no skip mode
        let allow_warped_motion = false; // intra frames have no warped motion

        // reduced_tx_set	f(1)
        let reduced_tx_set = buf.get_bit();

        // Global motion for intra frames is all-identity; decode_() handles this.
        let prev_gm = ctx.prev_gm_params;
        let global_motion_params = GlobalMotionParams::decode(
            buf,
            true,
            false,
            &prev_gm,
        );

        let film_grain_params = FilmGrainParams::decode(
            buf,
            sequence_header.film_grain_params_present,
            show_frame,
            showable_frame,
        );

        update_ref_frame_context(ctx, refresh_frame_flags, frame_type);

        Ok(FrameHeader {
            show_existing_frame,
            frame_type,
            show_frame,
            showable_frame,
            error_resilient_mode,
            disable_cdf_update,
            allow_screen_content_tools,
            force_integer_mv,
            current_frame_id,
            frame_size_override,
            frame_width: ctx.frame_width,
            frame_height: ctx.frame_height,
            upscaled_width: ctx.upscaled_width,
            render_width: ctx.render_width,
            render_height: ctx.render_height,
            order_hint,
            primary_ref_frame,
            refresh_frame_flags,
            ref_frame_idx,
            allow_high_precision_mv: false,
            interpolation_filter: None,
            is_motion_mode_switchable: false,
            use_ref_frame_mvs: false,
            allow_intrabc,
            disable_frame_end_update_cdf,
            tile_info,
            quantization_params,
            segmentation_params,
            delta_q_params,
            delta_lf_params,
            loop_filter_params,
            cdef_params,
            lr_params,
            tx_mode,
            reference_select,
            skip_mode_present,
            allow_warped_motion,
            reduced_tx_set,
            global_motion_params,
            temporal_point_info,
            buffer_removal_times,
            film_grain_params,
        })
    }

    /// Build a minimal FrameHeader for a show_existing_frame OBU.
    fn show_existing(
        _frame_to_show_idx: u8,
        frame_type: FrameType,
        refresh_frame_flags: u32,
        temporal_point_info: Option<TemporalPointInfo>,
    ) -> Self {
        Self {
            show_existing_frame: true,
            frame_type,
            show_frame: true,
            showable_frame: true,
            error_resilient_mode: false,
            disable_cdf_update: false,
            allow_screen_content_tools: false,
            force_integer_mv: false,
            current_frame_id: 0,
            frame_size_override: false,
            frame_width: 0,
            frame_height: 0,
            upscaled_width: 0,
            render_width: 0,
            render_height: 0,
            order_hint: 0,
            primary_ref_frame: PRIMARY_REF_NONE,
            refresh_frame_flags,
            ref_frame_idx: [0; REFS_PER_FRAME as usize],
            allow_high_precision_mv: false,
            interpolation_filter: None,
            is_motion_mode_switchable: false,
            use_ref_frame_mvs: false,
            allow_intrabc: false,
            disable_frame_end_update_cdf: false,
            tile_info: TileInfo {
                uniform_tile_spacing_flag: false,
                tile_cols_log2: 0,
                tile_rows_log2: 0,
                tile_cols: 1,
                tile_rows: 1,
                mi_col_starts: vec![0],
                mi_row_starts: vec![0],
                context_update_tile_id: 0,
                tile_size_bytes: 4,
            },
            quantization_params: QuantizationParams {
                base_q_idx: 0,
                delta_q_y_dc: 0,
                diff_uv_delta: false,
                delta_q_u_dc: 0,
                delta_q_u_ac: 0,
                delta_q_v_dc: 0,
                delta_q_v_ac: 0,
                using_qmatrix: false,
                qm_y: 0,
                qm_u: 0,
                qm_v: 0,
            },
            segmentation_params: SegmentationParams {
                segmentation_enabled: false,
                segmentation_update_map: false,
                segmentation_temporal_update: false,
                segmentation_update_data: false,
                features: core::array::from_fn(|_| core::array::from_fn(|_| SegmentFeature::default())),
            },
            delta_q_params: DeltaQParams {
                delta_q_present: false,
                delta_q_res: 0,
            },
            delta_lf_params: DeltaLfParams {
                delta_lf_present: false,
                delta_lf_res: 0,
                delta_lf_multi: false,
            },
            loop_filter_params: LoopFilterParams::default_params(),
            cdef_params: CdefParams::default_params(),
            lr_params: LrParams {
                lr_type: [RestorationType::None; 3],
                lr_unit_shift: 0,
                lr_uv_shift: false,
            },
            tx_mode: TxMode::Select,
            reference_select: false,
            skip_mode_present: false,
            allow_warped_motion: false,
            reduced_tx_set: false,
            global_motion_params: GlobalMotionParams {
                gm_type: [GmType::Identity; REFS_PER_FRAME as usize],
                gm_params: [[0; 6]; REFS_PER_FRAME as usize],
            },
            temporal_point_info,
            buffer_removal_times: vec![],
            film_grain_params: None,
        }
    }
}

/// Decode skip_mode_params().
///
/// AV1 spec Section 5.9.22 - skip_mode_params().
/// Skip mode signals a zero-residual inter frame coded with the optimal
/// compound prediction reference pair.
fn decode_skip_mode_params(
    buf: &mut Buffer,
    sequence_header: &crate::obu::sequence_header::SequenceHeader,
    ctx: &ObuContext,
    ref_frame_idx: &[u8; REFS_PER_FRAME as usize],
    frame_type: FrameType,
    reference_select: bool,
) -> bool {
    let skip_mode_allowed = if ctx.frame_is_intra
        || !reference_select
        || !sequence_header.enable_order_hint
    {
        false
    } else {
        // A full decoder would search for the two reference frames whose
        // order hints are closest to the current frame. For this analysis
        // tool we assume skip mode is allowed if the conditions above are met.
        let _ = (frame_type, ref_frame_idx);
        true
    };

    if skip_mode_allowed {
        // skip_mode_present	f(1)
        buf.get_bit()
    } else {
        false
    }
}

/// Update the decoder context with current frame information.
///
/// Called at the end of frame header parsing to store frame metadata in the
/// reference buffer slots indicated by refresh_frame_flags.
fn update_ref_frame_context(
    ctx: &mut ObuContext,
    refresh_frame_flags: u32,
    frame_type: FrameType,
) {
    for i in 0..NUM_REF_FRAMES as usize {
        if (refresh_frame_flags >> i) & 1 != 0 {
            if ctx.ref_frame_type.len() > i {
                ctx.ref_frame_type[i] = frame_type;
            }
            if ctx.ref_upscaled_width.len() > i {
                ctx.ref_upscaled_width[i] = ctx.upscaled_width;
                ctx.ref_frame_height[i] = ctx.frame_height;
                ctx.ref_render_width[i] = ctx.render_width;
                ctx.ref_render_height[i] = ctx.render_height;
                ctx.ref_order_hint[i] = ctx.order_hint;
            }
        }
    }
    // A full decoder would also persist global motion parameters here.
}
