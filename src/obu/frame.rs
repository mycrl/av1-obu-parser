/// AV1 Frame OBU parsing.
///
/// AV1 spec Section 5.12 - frame_obu().
/// A Frame OBU combines a FrameHeader OBU and a TileGroup OBU into a single
/// unit and is the most common way frames are encoded in practice.
use crate::buffer::Buffer;

use super::{ObuContext, ObuError, frame_header::FrameHeader, tile_group::TileGroup};

/// A complete AV1 frame (parsed from a Frame OBU).
///
/// A Frame OBU contains:
/// 1. The uncompressed frame header — encoding parameters for the frame.
/// 2. Byte-alignment padding bits.
/// 3. Tile group data — the actual encoded image data, read as tile boundaries
///    only (tile decoding is out of scope for this analysis tool).
#[derive(Debug, Clone)]
pub struct Frame {
    /// Parsed frame header with all frame-level coding parameters.
    pub header: FrameHeader,
    /// Tile group metadata (tile sizes and offsets; image data is not decoded).
    pub tile_group: TileGroup,
}

impl Frame {
    /// Parse a Frame OBU.
    ///
    /// AV1 spec Section 5.12 - frame_obu().
    ///
    /// Parsing order:
    /// 1. Parse the frame header (uncompressed_header).
    /// 2. Skip byte-alignment padding (byte_alignment).
    /// 3. Parse the tile group header (tile_group_obu).
    pub fn decode(ctx: &mut ObuContext, buf: &mut Buffer) -> Result<Self, ObuError> {
        let header = FrameHeader::decode(ctx, buf)?;

        // show_existing_frame carries no tile data.
        if header.show_existing_frame {
            return Ok(Frame {
                tile_group: TileGroup::empty(),
                header,
            });
        }

        // byte_alignment(): pad to a byte boundary between header and tile data.
        buf.byte_align();

        let num_tiles = header.tile_info.tile_cols * header.tile_info.tile_rows;
        let tile_group = TileGroup::decode(ctx, buf, &header.tile_info, num_tiles)?;

        Ok(Frame { header, tile_group })
    }
}
