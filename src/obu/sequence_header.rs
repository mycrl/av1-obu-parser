use super::{Buffer, ObuContext, ObuError, ObuUnknownError};

use crate::constants::{SELECT_INTEGER_MV, SELECT_SCREEN_CONTENT_TOOLS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorPrimaries {
    Bt709,
    Unspecified,
    Bt470M,
    Bt470BG,
    Bt601,
    Smpte240,
    GenericFilm,
    Bt2020,
    Xyz,
    Smpte431,
    Smpte432,
    Ebu3213,
}

impl TryFrom<u8> for ColorPrimaries {
    type Error = ObuError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            1 => Self::Bt709,
            2 => Self::Unspecified,
            4 => Self::Bt470M,
            5 => Self::Bt470BG,
            6 => Self::Bt601,
            7 => Self::Smpte240,
            8 => Self::GenericFilm,
            9 => Self::Bt2020,
            10 => Self::Xyz,
            11 => Self::Smpte431,
            12 => Self::Smpte432,
            22 => Self::Ebu3213,
            _ => return Err(ObuError::Unknown(ObuUnknownError::ColorPrimaries)),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferCharacteristics {
    Bt709,
    Unspecified,
    Bt470M,
    Bt470BG,
    Bt601,
    Smpte240,
    Linear,
    Log100,
    Log100Sqrt10,
    Iec61966,
    Bt1361,
    Srgb,
    Bt202010Bit,
    Bt202012Bit,
    Smpte2084,
    Smpte428,
    Hlg,
}

impl TryFrom<u8> for TransferCharacteristics {
    type Error = ObuError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            1 => Self::Bt709,
            2 => Self::Unspecified,
            4 => Self::Bt470M,
            5 => Self::Bt470BG,
            6 => Self::Bt601,
            7 => Self::Smpte240,
            8 => Self::Linear,
            9 => Self::Log100,
            10 => Self::Log100Sqrt10,
            11 => Self::Iec61966,
            12 => Self::Bt1361,
            13 => Self::Srgb,
            14 => Self::Bt202010Bit,
            15 => Self::Bt202012Bit,
            16 => Self::Smpte2084,
            17 => Self::Smpte428,
            18 => Self::Hlg,
            _ => return Err(ObuError::Unknown(ObuUnknownError::TransferCharacteristics)),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixCoefficients {
    Identity,
    Bt709,
    Unspecified,
    Fcc,
    Bt470BG,
    Bt601,
    Smpte240,
    SmpteYcgco,
    Bt2020Ncl,
    Bt2020Cl,
    Smpte2085,
    ChromatNcl,
    ChromatCl,
    Ictcp,
}

impl TryFrom<u8> for MatrixCoefficients {
    type Error = ObuError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::Identity,
            1 => Self::Bt709,
            2 => Self::Unspecified,
            4 => Self::Fcc,
            5 => Self::Bt470BG,
            6 => Self::Bt601,
            7 => Self::Smpte240,
            8 => Self::SmpteYcgco,
            9 => Self::Bt2020Ncl,
            10 => Self::Bt2020Cl,
            11 => Self::Smpte2085,
            12 => Self::ChromatNcl,
            13 => Self::ChromatCl,
            14 => Self::Ictcp,
            _ => return Err(ObuError::Unknown(ObuUnknownError::MatrixCoefficients)),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaSamplePosition {
    Unknown,
    Vertical,
    Colocated,
}

impl TryFrom<u8> for ChromaSamplePosition {
    type Error = ObuError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::Unknown,
            1 => Self::Vertical,
            2 => Self::Colocated,
            _ => return Err(ObuError::Unknown(ObuUnknownError::ChromaSamplePosition)),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ColorConfig {
    pub high_bitdepth: bool,
    pub twelve_bit: bool,
    pub mono_chrome: bool,
    pub color_description_present: bool,
    pub color_primaries: ColorPrimaries,
    pub transfer_characteristics: TransferCharacteristics,
    pub matrix_coefficients: MatrixCoefficients,
    pub color_range: bool,
    pub subsampling_x: bool,
    pub subsampling_y: bool,
    pub chroma_sample_position: Option<ChromaSamplePosition>,
    pub separate_uv_delta_q: bool,
}

impl ColorConfig {
    pub fn decode(
        ctx: &mut ObuContext,
        buf: &mut Buffer,
        profile: SequenceProfile,
    ) -> Result<Self, ObuError> {
        // high_bitdepth	f(1)
        let high_bitdepth = buf.get_bit();

        let mut twelve_bit = false;
        ctx.bit_depth = if profile == SequenceProfile::Professional && high_bitdepth {
            // twelve_bit	f(1)
            twelve_bit = buf.get_bit();
            if twelve_bit {
                12
            } else {
                10
            }
        } else {
            if high_bitdepth {
                10
            } else {
                8
            }
        };

        let mono_chrome = if profile == SequenceProfile::High {
            false
        } else {
            // mono_chrome	f(1)
            buf.get_bit()
        };

        ctx.num_planes = if mono_chrome { 1 } else { 3 };

        // color_description_present_flag	f(1)
        let color_description_present = buf.get_bit();
        let (color_primaries, transfer_characteristics, matrix_coefficients) =
            if color_description_present {
                (
                    // color_primaries	f(8)
                    ColorPrimaries::try_from(buf.get_bits(8) as u8)?,
                    // transfer_characteristics	f(8)
                    TransferCharacteristics::try_from(buf.get_bits(8) as u8)?,
                    // matrix_coefficients	f(8)
                    MatrixCoefficients::try_from(buf.get_bits(8) as u8)?,
                )
            } else {
                (
                    ColorPrimaries::Unspecified,
                    TransferCharacteristics::Unspecified,
                    MatrixCoefficients::Unspecified,
                )
            };

        let mut color_range = false;
        let mut subsampling_x = false;
        let mut subsampling_y = false;
        let mut chroma_sample_position = None;

        if mono_chrome {
            // color_range f(1)
            color_range = buf.get_bit();
            subsampling_x = true;
            subsampling_y = true;
            chroma_sample_position = Some(ChromaSamplePosition::Unknown);

            return Ok(Self {
                high_bitdepth,
                twelve_bit,
                mono_chrome,
                color_description_present,
                color_primaries,
                transfer_characteristics,
                matrix_coefficients,
                color_range,
                subsampling_x,
                subsampling_y,
                chroma_sample_position,
                separate_uv_delta_q: false,
            });
        }

        if color_primaries == ColorPrimaries::Bt709
            && transfer_characteristics == TransferCharacteristics::Srgb
            && matrix_coefficients == MatrixCoefficients::Identity
        {
            color_range = true;
            subsampling_x = false;
            subsampling_y = false;
        } else {
            // color_range f(1)
            color_range = buf.get_bit();
            if profile == SequenceProfile::Main {
                subsampling_x = true;
                subsampling_y = true;
            } else if profile == SequenceProfile::High {
                subsampling_x = false;
                subsampling_y = false;
            } else {
                if ctx.bit_depth == 12 {
                    // subsampling_x	f(1)
                    subsampling_x = buf.get_bit();
                    subsampling_y = if subsampling_x {
                        // subsampling_y	f(1)
                        buf.get_bit()
                    } else {
                        false
                    };
                } else {
                    subsampling_x = true;
                    subsampling_y = false;
                }
            }

            if subsampling_x && subsampling_y {
                // chroma_sample_position	f(2)
                chroma_sample_position =
                    Some(ChromaSamplePosition::try_from(buf.get_bits(2) as u8)?);
            }
        };

        // separate_uv_delta_q	f(1)
        let separate_uv_delta_q = buf.get_bit();

        Ok(Self {
            high_bitdepth,
            twelve_bit,
            mono_chrome,
            color_description_present,
            color_primaries,
            transfer_characteristics,
            matrix_coefficients,
            color_range,
            subsampling_x,
            subsampling_y,
            chroma_sample_position,
            separate_uv_delta_q,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceProfile {
    Main,
    High,
    Professional,
}

impl TryFrom<u8> for SequenceProfile {
    type Error = ObuError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::Main,
            1 => Self::High,
            2 => Self::Professional,
            _ => return Err(ObuError::Unknown(ObuUnknownError::Profile)),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EqualPictureInterval {
    pub num_ticks_per_picture: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct TimingInfo {
    pub num_units_in_display_tick: u32,
    pub time_scale: u32,
    pub equal_picture_interval: Option<EqualPictureInterval>,
}

impl TimingInfo {
    pub fn decode(buf: &mut Buffer<'_>) -> Self {
        // num_units_in_display_tick f(32)
        let num_units_in_display_tick = buf.get_bits(32);

        // time_scale f(32)
        let time_scale = buf.get_bits(32);

        // equal_picture_interval f(1)
        let equal_picture_interval = if buf.get_bit() {
            Some(EqualPictureInterval {
                // num_ticks_per_picture_minus_1 uvlc()
                num_ticks_per_picture: buf.get_uvlc() + 1,
            })
        } else {
            None
        };

        Self {
            num_units_in_display_tick,
            time_scale,
            equal_picture_interval,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DecoderModelInfo {
    pub buffer_delay_length: u8,
    pub num_units_in_decoding_tick: u32,
    pub buffer_removal_time_length: u8,
    pub frame_presentation_time_length: u8,
}

impl DecoderModelInfo {
    pub fn decode(buf: &mut Buffer<'_>) -> Self {
        Self {
            // buffer_delay_length_minus_1 f(5)
            buffer_delay_length: buf.get_bits(5) as u8 + 1,
            // num_units_in_decoding_tick f(32)
            num_units_in_decoding_tick: buf.get_bits(32),
            // buffer_removal_time_length_minus_1 f(5)
            buffer_removal_time_length: buf.get_bits(5) as u8 + 1,
            // frame_presentation_time_length_minus_1 f(5)
            frame_presentation_time_length: buf.get_bits(5) as u8 + 1,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OperatingParametersInfo {
    pub decoder_buffer_delay: u32,
    pub encoder_buffer_delay: u32,
    pub low_delay_mode_flag: bool,
}

impl OperatingParametersInfo {
    pub fn decode(buf: &mut Buffer<'_>, decoder_model_info: &DecoderModelInfo) -> Self {
        let size = decoder_model_info.buffer_delay_length as usize;
        Self {
            // decoder_buffer_delay[ op ]	f(n)
            decoder_buffer_delay: buf.get_bits(size),
            // encoder_buffer_delay[ op ]	f(n)
            encoder_buffer_delay: buf.get_bits(size),
            // low_delay_mode_flag[ op ]	f(1)
            low_delay_mode_flag: buf.get_bit(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OperatingPoint {
    pub idc: u16,
    pub level_idx: u8,
    pub tier: bool,
    pub operating_parameters_info: Option<OperatingParametersInfo>,
    pub initial_display_delay: u8,
}

#[derive(Debug, Clone)]
pub struct FrameIdNumbersPresent {
    pub delta_frame_id_length: u8,
    pub additional_frame_id_length: u8,
}

impl FrameIdNumbersPresent {
    pub fn decode(buf: &mut Buffer<'_>) -> Self {
        Self {
            // delta_frame_id_length_minus_2	f(4)
            delta_frame_id_length: buf.get_bits(4) as u8 + 2,
            // additional_frame_id_length_minus_1	f(3)
            additional_frame_id_length: buf.get_bits(4) as u8 + 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SequenceHeader {
    pub seq_profile: SequenceProfile,
    pub still_picture: bool,
    pub reduced_still_picture_header: bool,
    pub timing_info: Option<TimingInfo>,
    pub decoder_model_info: Option<DecoderModelInfo>,
    pub initial_display_delay_present_flag: bool,
    pub operating_points: Vec<OperatingPoint>,
    pub frame_width_bits: u8,
    pub frame_height_bits: u8,
    pub max_frame_width: u16,
    pub max_frame_height: u16,
    pub frame_id_numbers_present: Option<FrameIdNumbersPresent>,
    pub use_128x128_superblock: bool,
    pub enable_filter_intra: bool,
    pub enable_intra_edge_filter: bool,
    pub enable_interintra_compound: bool,
    pub enable_masked_compound: bool,
    pub enable_warped_motion: bool,
    pub enable_dual_filter: bool,
    pub enable_order_hint: bool,
    pub enable_jnt_comp: bool,
    pub enable_ref_frame_mvs: bool,
    pub seq_choose_screen_content_tools: bool,
    pub seq_force_screen_content_tools: u8,
    pub seq_force_integer_mv: u8,
    pub enable_superres: bool,
    pub enable_cdef: bool,
    pub enable_restoration: bool,
    pub color_config: ColorConfig,
    pub film_grain_params_present: bool,
}

impl SequenceHeader {
    pub fn decode(ctx: &mut ObuContext, buf: &mut Buffer) -> Result<Self, ObuError> {
        // seq_profile f(3)
        let seq_profile = SequenceProfile::try_from(buf.get_bits(3) as u8)?;

        // still_picture f(1)
        let still_picture = buf.get_bit();

        // reduced_still_picture_header f(1)
        let reduced_still_picture_header = buf.get_bit();

        let mut timing_info = None;
        let mut decoder_model_info_present_flag = false;
        let mut decoder_model_info = None;
        let mut initial_display_delay_present_flag = false;
        let mut operating_points = Vec::with_capacity(32);

        if reduced_still_picture_header {
            operating_points.push(OperatingPoint {
                idc: 0,
                // seq_level_idx[ 0 ] f(5)
                level_idx: buf.get_bits(5) as u8,
                tier: false,
                operating_parameters_info: None,
                initial_display_delay: 10,
            });
        } else {
            // timing_info_present_flag f(1)
            let timing_info_present_flag = buf.get_bit();
            if timing_info_present_flag {
                timing_info = Some(TimingInfo::decode(buf.as_mut()));

                // decoder_model_info_present_flag f(1)
                decoder_model_info_present_flag = buf.get_bit();
                if decoder_model_info_present_flag {
                    decoder_model_info = Some(DecoderModelInfo::decode(buf.as_mut()));
                }
            }

            // initial_display_delay_present_flag	f(1)
            initial_display_delay_present_flag = buf.get_bit();

            // operating_points_cnt_minus_1	f(5)
            let operating_points_cnt = buf.get_bits(5) as u8 + 1;
            for _ in 0..operating_points_cnt as usize {
                // operating_point_idc[ i ]	f(12)
                let idc = buf.get_bits(12) as u16;

                // seq_level_idx[ i ]	f(5)
                let level_idx = buf.get_bits(5) as u8;
                let tier = if level_idx > 7 {
                    // seq_tier[ i ]	f(1)
                    buf.get_bit()
                } else {
                    false
                };

                let mut operating_parameters_info = None;
                if decoder_model_info_present_flag {
                    // decoder_model_present_for_this_op[ i ]	f(1)
                    let decoder_model_present = buf.get_bit();
                    if decoder_model_present {
                        operating_parameters_info = Some(OperatingParametersInfo::decode(
                            buf.as_mut(),
                            &decoder_model_info.unwrap(),
                        ));
                    }
                }

                let initial_display_delay = if initial_display_delay_present_flag {
                    // initial_display_delay_present_for_this_op[ i ]	f(1)
                    let initial_display_delay_present = buf.get_bit();
                    if initial_display_delay_present {
                        // initial_display_delay_minus_1[ i ]	f(4)
                        buf.get_bits(4) as u8 + 1
                    } else {
                        10
                    }
                } else {
                    10
                };

                operating_points.push(OperatingPoint {
                    idc,
                    level_idx,
                    tier,
                    operating_parameters_info,
                    initial_display_delay,
                });
            }
        }

        ctx.operating_point_idc =
            operating_points[if ctx.operating_point < operating_points.len() {
                ctx.operating_point
            } else {
                0
            }]
            .idc;

        // frame_width_bits_minus_1	f(4)
        let frame_width_bits = buf.get_bits(4) as u8 + 1;

        // frame_height_bits_minus_1	f(4)
        let frame_height_bits = buf.get_bits(4) as u8 + 1;

        // max_frame_width_minus_1	f(n)
        let max_frame_width = buf.get_bits(frame_width_bits as usize) as u16;

        // max_frame_height_minus_1	f(n)
        let max_frame_height = buf.get_bits(frame_height_bits as usize) as u16;

        let frame_id_numbers_present = if !reduced_still_picture_header {
            // frame_id_numbers_present_flag	f(1)
            if buf.get_bit() {
                Some(FrameIdNumbersPresent::decode(buf.as_mut()))
            } else {
                None
            }
        } else {
            None
        };

        // use_128x128_superblock	f(1)
        let use_128x128_superblock = buf.get_bit();

        // enable_filter_intra	f(1)
        let enable_filter_intra = buf.get_bit();

        // enable_intra_edge_filter	f(1)
        let enable_intra_edge_filter = buf.get_bit();

        let mut enable_interintra_compound = false;
        let mut enable_masked_compound = false;
        let mut enable_warped_motion = false;
        let mut enable_dual_filter = false;
        let mut enable_order_hint = false;
        let mut enable_jnt_comp = false;
        let mut enable_ref_frame_mvs = false;
        let mut seq_choose_screen_content_tools = false;
        let mut seq_force_screen_content_tools = SELECT_SCREEN_CONTENT_TOOLS;
        let mut seq_force_integer_mv = SELECT_INTEGER_MV;

        if reduced_still_picture_header {
            ctx.order_hint_bits = 0;
        } else {
            // enable_interintra_compound	f(1)
            enable_interintra_compound = buf.get_bit();

            // enable_masked_compound	f(1)
            enable_masked_compound = buf.get_bit();

            // enable_warped_motion	f(1)
            enable_warped_motion = buf.get_bit();

            // enable_dual_filter	f(1)
            enable_dual_filter = buf.get_bit();

            // enable_order_hint	f(1)
            enable_order_hint = buf.get_bit();
            if enable_order_hint {
                // enable_jnt_comp	f(1)
                enable_jnt_comp = buf.get_bit();

                // enable_ref_frame_mvs	f(1)
                enable_ref_frame_mvs = buf.get_bit();
            }

            // seq_choose_screen_content_tools	f(1)
            seq_choose_screen_content_tools = buf.get_bit();
            if !seq_choose_screen_content_tools {
                // seq_force_screen_content_tools	f(1)
                seq_force_screen_content_tools = buf.get_bit() as u8;
            }

            if seq_force_screen_content_tools > 0 {
                // seq_choose_integer_mv	f(1)
                let seq_choose_integer_mv = buf.get_bit();
                if !seq_choose_integer_mv {
                    // seq_force_integer_mv	f(1)
                    seq_force_integer_mv = buf.get_bit() as u8;
                }
            }

            ctx.order_hint_bits = if enable_order_hint {
                // order_hint_bits_minus_1	f(3)
                buf.get_bits(3) as usize + 1
            } else {
                0
            };
        }

        // enable_superres	f(1)
        let enable_superres = buf.get_bit();

        // enable_cdef	f(1)
        let enable_cdef = buf.get_bit();

        // enable_restoration	f(1)
        let enable_restoration = buf.get_bit();

        let color_config = ColorConfig::decode(ctx, buf, seq_profile)?;

        // film_grain_params_present	f(1)
        let film_grain_params_present = buf.get_bit();

        Ok(Self {
            seq_profile,
            still_picture,
            reduced_still_picture_header,
            timing_info,
            decoder_model_info,
            initial_display_delay_present_flag,
            operating_points,
            frame_width_bits,
            frame_height_bits,
            max_frame_width,
            max_frame_height,
            frame_id_numbers_present,
            use_128x128_superblock,
            enable_filter_intra,
            enable_intra_edge_filter,
            enable_interintra_compound,
            enable_masked_compound,
            enable_warped_motion,
            enable_dual_filter,
            enable_order_hint,
            enable_jnt_comp,
            enable_ref_frame_mvs,
            seq_choose_screen_content_tools,
            seq_force_screen_content_tools,
            seq_force_integer_mv,
            enable_superres,
            enable_cdef,
            enable_restoration,
            color_config,
            film_grain_params_present,
        })
    }
}
