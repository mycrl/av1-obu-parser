pub mod frame;
pub mod frame_header;
pub mod metadata;
pub mod sequence_header;
pub mod tile_group;
pub mod tile_list;

use frame::Frame;
use frame_header::{FrameHeader, FrameType};
use sequence_header::SequenceHeader;

use crate::{buffer::Buffer, constants::NUM_REF_FRAMES};

/// see: https://aomediacodec.github.io/av1-spec/#obu-header-semantics
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObuType {
    Reserved(u8),
    SequenceHeader,
    /// Note: The temporal delimiter has an empty payload.
    TemporalDelimiter,
    FrameHeader,
    TileGroup,
    Metadata,
    Frame,
    RedundantFrameHeader,
    TileList,
    /// Note: obu_padding_length is not coded in the bitstream but can be computed based on
    /// obu_size minus the number of trailing bytes. In practice, though, since this is padding
    /// data meant to be skipped, decoders do not need to determine either that length nor the
    /// number of trailing bytes. They can ignore the entire OBU. Ignoring the OBU can be done
    /// based on obu_size. The last byte of the valid content of the payload data for this OBU type
    /// is considered to be the last byte that is not equal to zero. This rule is to prevent the
    /// dropping of valid bytes by systems that interpret trailing zero bytes as a continuation of
    /// the trailing bits in an OBU. This implies that when any payload data is present for this
    /// OBU type, at least one byte of the payload data (including the trailing bit) shall not be
    /// equal to 0.
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

/// https://aomediacodec.github.io/av1-spec/#obu-extension-header-syntax
#[derive(Debug, Clone, Copy)]
pub struct ObuHeaderExtension {
    pub temporal_id: u8,
    pub spatial_id: u8,
}

impl ObuHeaderExtension {
    pub fn decode(buf: &mut Buffer<'_>) -> Result<Self, ObuError> {
        // temporal_id f(3)
        let temporal_id = buf.get_bits(3) as u8;

        // spatial_id f(2)
        let spatial_id = buf.get_bits(2) as u8;

        // extension_header_reserved_3bits
        buf.seek_bits(3);

        Ok(Self {
            temporal_id,
            spatial_id,
        })
    }
}

/// see: https://aomediacodec.github.io/av1-spec/#obu-header-syntax
#[derive(Debug, Clone, Copy)]
pub struct ObuHeader {
    pub r#type: ObuType,
    pub has_size: bool,
    pub extension: Option<ObuHeaderExtension>,
}

impl ObuHeader {
    pub fn decode(buf: &mut Buffer<'_>) -> Result<Self, ObuError> {
        // obu_forbidden_bit f(1)
        buf.seek_bits(1);

        // obu_type f(4)
        let r#type = ObuType::try_from(buf.get_bits(4) as u8)?;

        // obu_extension_flag f(1)
        let obu_extension_flag = buf.get_bit();

        // obu_has_size_field f(1)
        let has_size = buf.get_bit();

        // obu_reserved_1bit
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

#[derive(Debug)]
pub enum Obu {
    SequenceHeader(SequenceHeader),
    Frame(Frame),
    TemporalDelimiter,
    Drop,
}

#[derive(Default)]
/// Open Bitstream Unit Parser
///
/// see: https://aomediacodec.github.io/av1-spec/#obu-syntax
pub struct ObuParser {
    pub ctx: ObuContext,
}

impl ObuParser {
    pub fn parse(&mut self, buf: &mut Buffer) -> Result<Obu, ObuError> {
        let header = ObuHeader::decode(buf.as_mut())?;
        let size = if header.has_size {
            // obu_size leb128()
            Some(buf.get_leb128() as usize)
        } else {
            None
        };

        if header.r#type != ObuType::SequenceHeader
            && header.r#type != ObuType::TemporalDelimiter
            && self.ctx.operating_point_idc != 0
        {
            if let Some(ext) = header.extension {
                let in_temporal_layer = (1 >> ext.temporal_id) & 1;
                let in_spatial_layer = (1 >> (ext.spatial_id + 8)) & 1;
                if in_temporal_layer == 0 || in_spatial_layer == 0 {
                    return Ok(Obu::Drop);
                }
            }
        }

        Ok(match header.r#type {
            ObuType::SequenceHeader => {
                Obu::SequenceHeader(SequenceHeader::decode(&mut self.ctx, buf)?)
            }
            ObuType::Frame => Obu::Frame(Frame::decode(&mut self.ctx, buf)?),
            ObuType::TemporalDelimiter => Obu::TemporalDelimiter,
            _ => todo!(),
        })
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObuError {
    Unknown(ObuUnknownError),
    NotFoundSequenceHeader,
}

impl std::error::Error for ObuError {}

impl std::fmt::Display for ObuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}

#[derive(Default, Debug)]
pub struct ObuContext {
    pub sequence_header: Option<SequenceHeader>,
    pub obu_header_extension: Option<ObuHeaderExtension>,
    pub num_planes: u8,
    pub seen_frame_header: bool,
    pub frame_is_intra: bool,
    pub order_hint: u32,
    pub frame_width: u16,
    pub frame_height: u16,
    pub superres_denom: u8,
    pub upscaled_width: u16,
    pub mi_cols: u32,
    pub mi_rows: u32,
    pub render_width: u16,
    pub render_height: u16,
    pub delta_frame_id: u32,
    pub bit_depth: u8,
    pub order_hint_bits: usize,
    pub operating_point: usize,
    pub operating_point_idc: u16,
    pub frame_type_refs: Vec<FrameType>,
}
