use super::Buffer;

#[derive(Debug, Clone, Copy)]
pub struct TileListEntry {
    pub anchor_frame_idx: u8,
    pub anchor_tile_row: u8,
    pub anchor_tile_col: u8,
    pub tile_data_size: u16,
    pub coded_tile_data: u32,
}

impl TileListEntry {
    pub fn decode(buf: &mut Buffer) -> Self {
        // anchor_frame_idx	f(8)
        let anchor_frame_idx = buf.get_bits(8) as u8;

        // anchor_tile_row	f(8)
        let anchor_tile_row = buf.get_bits(8) as u8;

        // anchor_tile_col	f(8)
        let anchor_tile_col = buf.get_bits(8) as u8;

        // tile_data_size_minus_1	f(16)
        let tile_data_size = buf.get_bits(16) as u16 + 1;

        // coded_tile_data	f(N)
        let coded_tile_data = buf.get_bits(8 * tile_data_size as usize);
        Self {
            anchor_frame_idx,
            anchor_tile_col,
            anchor_tile_row,
            tile_data_size,
            coded_tile_data,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TileList {
    pub output_frame_width_in_tiles: u8,
    pub output_frame_height_in_tiles: u8,
    pub tile_list_entrys: Vec<TileListEntry>,
}

impl TileList {
    pub fn decode(buf: &mut Buffer) -> Self {
        // output_frame_width_in_tiles_minus_1	f(8)
        let output_frame_width_in_tiles = buf.get_bits(8) as u8;

        // output_frame_height_in_tiles_minus_1	f(8)
        let output_frame_height_in_tiles = buf.get_bits(8) as u8;

        // tile_count_minus_1	f(16)
        let tile_count = buf.get_bits(16) as usize;
        let mut tile_list_entrys = Vec::with_capacity(tile_count);
        for _ in 0..tile_count {
            tile_list_entrys.push(TileListEntry::decode(buf));
        }

        Self {
            output_frame_height_in_tiles,
            output_frame_width_in_tiles,
            tile_list_entrys,
        }
    }
}
