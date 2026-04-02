/// AV1 TileGroup OBU parsing.
///
/// AV1 spec Section 5.11.1 - tile_group_obu().
/// A TileGroup OBU holds encoded data for a contiguous range of tiles within
/// a frame. Tiles are rectangular partitions that can be decoded independently.
///
/// This module is an analysis tool: it parses tile header information (index,
/// size, offset) but does not decode the actual image data inside each tile.
use crate::buffer::Buffer;

use super::{ObuContext, ObuError, frame_header::TileInfo};

/// Metadata for a single tile within a TileGroup.
#[derive(Debug, Clone)]
pub struct TileGroupHeader {
    /// Linear tile index (row-major order).
    pub tile_num: usize,
    /// Row index of this tile within the tile grid.
    pub tile_row: usize,
    /// Column index of this tile within the tile grid.
    pub tile_col: usize,
    /// Byte length of the encoded tile data (excluding this header).
    pub tile_size: usize,
    /// Byte offset of the tile data relative to the start of the TileGroup OBU payload.
    pub data_offset: usize,
}

/// Parsed TileGroup OBU.
///
/// Records metadata for every tile in the group. Tile image data is not decoded.
#[derive(Debug, Clone)]
pub struct TileGroup {
    /// Index of the first tile in this group.
    pub tg_start: usize,
    /// Index of the last tile in this group (inclusive).
    pub tg_end: usize,
    /// Per-tile header metadata.
    pub tiles: Vec<TileGroupHeader>,
}

impl TileGroup {
    /// Create an empty TileGroup (used when show_existing_frame is set).
    pub fn empty() -> Self {
        Self {
            tg_start: 0,
            tg_end: 0,
            tiles: Vec::new(),
        }
    }

    /// Parse a TileGroup OBU (or the embedded tile data inside a Frame OBU).
    ///
    /// AV1 spec Section 5.11.1 - tile_group_obu().
    ///
    /// Steps:
    /// 1. Read tile_start_and_end_present_flag to determine the tile range.
    /// 2. For each tile, read its declared size.
    /// 3. Skip the encoded tile data (not decoded by this tool).
    pub fn decode(
        ctx: &mut ObuContext,
        buf: &mut Buffer,
        tile_info: &TileInfo,
        num_tiles: usize,
    ) -> Result<Self, ObuError> {
        let mut tg_start = 0usize;
        let mut tg_end = num_tiles - 1;

        // When there is more than one tile the group range is optionally signalled.
        if num_tiles > 1 {
            // tile_start_and_end_present_flag	f(1)
            let tile_start_and_end_present = buf.get_bit();
            if tile_start_and_end_present {
                let tile_bits = tile_info.tile_cols_log2 + tile_info.tile_rows_log2;
                // tg_start	f(tileBits)
                tg_start = buf.get_bits(tile_bits) as usize;
                // tg_end	f(tileBits)
                tg_end = buf.get_bits(tile_bits) as usize;
            }
        }

        // Tile data begins on a byte boundary.
        buf.byte_align();

        let mut tiles = Vec::with_capacity(tg_end - tg_start + 1);

        for tile_num in tg_start..=tg_end {
            let tile_row = tile_num / tile_info.tile_cols;
            let tile_col = tile_num % tile_info.tile_cols;
            let last_tile = tile_num == tg_end;

            let (tile_size, data_offset) = if !last_tile {
                // tile_size_minus_1	le(tile_size_bytes)
                // tile_size_bytes is signalled in the frame header tile_info.
                let tile_size = buf.get_le(tile_info.tile_size_bytes) as usize + 1;
                let offset = buf.bytes_consumed();
                (tile_size, offset)
            } else {
                // The last tile's size is implicit (remainder of the OBU).
                let offset = buf.bytes_consumed();
                (0, offset)
            };

            tiles.push(TileGroupHeader {
                tile_num,
                tile_row,
                tile_col,
                tile_size,
                data_offset,
            });

            // Skip the encoded tile data — this tool does not decode image data.
            if !last_tile && tile_size > 0 {
                buf.seek_bits(tile_size * 8);
            }

            ctx.tile_num = tile_num;
        }

        // Reaching the last tile of the last group resets seen_frame_header,
        // allowing the next frame's header to be decoded fresh.
        if tg_end == num_tiles - 1 {
            ctx.seen_frame_header = false;
        }

        Ok(Self {
            tg_start,
            tg_end,
            tiles,
        })
    }
}
