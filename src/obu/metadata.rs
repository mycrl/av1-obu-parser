use super::{Buffer, ObuError, ObuUnknownError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataType {
    HdrCll,
    HdrMdcv,
    Scalability,
    ItutT35,
    Timecode,
    UnregisteredUserPrivate,
}

impl TryFrom<u8> for MetadataType {
    type Error = ObuError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            1 => Self::HdrCll,
            2 => Self::HdrMdcv,
            3 => Self::Scalability,
            4 => Self::ItutT35,
            5 => Self::Timecode,
            6..=31 => Self::UnregisteredUserPrivate,
            _ => return Err(ObuError::Unknown(ObuUnknownError::ChromaSamplePosition)),
        })
    }
}

#[derive(Debug, Clone)]
pub struct SpatialLayer {
    pub max_width: u16,
    pub max_height: u16,
}

#[derive(Debug, Clone)]
pub struct TemporalGroup {
    pub temporal_id: u8,
    pub temporal_switching_up_point: bool,
    pub spatial_switching_up_point: bool,
    pub ref_pic_diffs: Vec<u8>,
}

impl TemporalGroup {
    pub fn decode(buf: &mut Buffer) -> Self {
        // temporal_group_temporal_id[ i ]	f(3)
        let temporal_id = buf.get_bits(3) as u8;

        // temporal_group_temporal_switching_up_point_flag[ i ]	f(1)
        let temporal_switching_up_point = buf.get_bit();

        // temporal_group_spatial_switching_up_point_flag[ i ]	f(1)
        let spatial_switching_up_point = buf.get_bit();

        // temporal_group_ref_cnt[ i ]	f(3)
        let ref_cnt = buf.get_bits(3) as usize;
        let mut ref_pic_diffs = Vec::with_capacity(ref_cnt);
        for _ in 0..ref_cnt {
            ref_pic_diffs.push(buf.get_bits(8) as u8);
        }

        Self {
            temporal_id,
            temporal_switching_up_point,
            spatial_switching_up_point,
            ref_pic_diffs,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScalabilityStructure {
    pub spatial_layers: Option<Vec<SpatialLayer>>,
    pub spatial_layer_descriptions: Option<Vec<u8>>,
    pub temporal_groups: Option<Vec<TemporalGroup>>,
}

impl ScalabilityStructure {
    pub fn decode(buf: &mut Buffer) -> Self {
        // spatial_layers_cnt_minus_1	f(2)
        let spatial_layers_cnt = buf.get_bits(2) as usize + 1;

        // spatial_layer_dimensions_present_flag	f(1)
        let spatial_layer_dimensions_present = buf.get_bit();

        // spatial_layer_description_present_flag	f(1)
        let spatial_layer_description_present = buf.get_bit();

        // temporal_group_description_present_flag	f(1)
        let temporal_group_description_present = buf.get_bit();

        // scalability_structure_reserved_3bits	f(3)
        buf.seek_bits(3);

        let spatial_layers = if spatial_layer_dimensions_present {
            let mut spatial_layers = Vec::with_capacity(spatial_layers_cnt);
            for _ in 0..spatial_layers_cnt {
                spatial_layers.push(SpatialLayer {
                    // spatial_layer_max_width[ i ]	f(16)
                    max_width: buf.get_bits(16) as u16,
                    // spatial_layer_max_height[ i ]	f(16)
                    max_height: buf.get_bits(16) as u16,
                })
            }

            Some(spatial_layers)
        } else {
            None
        };

        let spatial_layer_descriptions = if spatial_layer_description_present {
            let mut spatial_layer_ref_ids = Vec::with_capacity(spatial_layers_cnt);
            for _ in 0..spatial_layers_cnt {
                spatial_layer_ref_ids.push(
                    // spatial_layer_ref_id[ i ]	f(8)
                    buf.get_bits(8) as u8,
                )
            }

            Some(spatial_layer_ref_ids)
        } else {
            None
        };

        let temporal_groups = if temporal_group_description_present {
            // temporal_group_size	f(8)
            let temporal_group_size = buf.get_bits(8) as usize;
            let mut temporal_groups = Vec::with_capacity(temporal_group_size);
            for _ in 0..temporal_group_size {
                temporal_groups.push(TemporalGroup::decode(buf));
            }

            Some(temporal_groups)
        } else {
            None
        };

        Self {
            spatial_layers,
            spatial_layer_descriptions,
            temporal_groups,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalabilityModeIdc {
    L1T2,
    L1T3,
    L2T1,
    L2T2,
    L2T3,
    S2T1,
    S2T2,
    S2T3,
    L2T1h,
    L2T2h,
    L2T3h,
    S2T1h,
    S2T2h,
    S2T3h,
    SS,
    L3T1,
    L3T2,
    L3T3,
    S3T1,
    S3T2,
    S3T3,
    L3T2Key,
    L3T3Key,
    L4T5Key,
    L4T7Key,
    L3T2KeyShift,
    L3T3KeyShift,
    L4T5KeyShift,
    L4T7KeyShift,
}

impl TryFrom<u8> for ScalabilityModeIdc {
    type Error = ObuError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::L1T2,
            1 => Self::L1T3,
            2 => Self::L2T1,
            3 => Self::L2T2,
            4 => Self::L2T3,
            5 => Self::S2T1,
            6 => Self::S2T2,
            7 => Self::S2T3,
            8 => Self::L2T1h,
            9 => Self::L2T2h,
            10 => Self::L2T3h,
            11 => Self::S2T1h,
            12 => Self::S2T2h,
            13 => Self::S2T3h,
            14 => Self::SS,
            15 => Self::L3T1,
            16 => Self::L3T2,
            17 => Self::L3T3,
            18 => Self::S3T1,
            19 => Self::S3T2,
            20 => Self::S3T3,
            21 => Self::L3T2Key,
            22 => Self::L3T3Key,
            23 => Self::L4T5Key,
            24 => Self::L4T7Key,
            25 => Self::L3T2KeyShift,
            26 => Self::L3T3KeyShift,
            27 => Self::L4T5KeyShift,
            28 => Self::L4T7KeyShift,
            _ => return Err(ObuError::Unknown(ObuUnknownError::ScalabilityModeIdc)),
        })
    }
}

#[derive(Debug, Clone)]
pub enum Metadata {
    UnregisteredUserPrivate(u8),
    HdrCll {
        max_cll: u16,
        max_fall: u16,
    },
    HdrMdcv {
        primary_chromaticity_x: [u16; 3],
        primary_chromaticity_y: [u16; 3],
        white_point_chromaticity_x: u16,
        white_point_chromaticity_y: u16,
        luminance_max: u32,
        luminance_min: u32,
    },
    Scalability {
        mode_idc: ScalabilityModeIdc,
        scalability_structure: Option<ScalabilityStructure>,
    },
    ItutT35 {
        country_code: u8,
        country_code_extension_byte: Option<u8>,
    },
    Timecode {
        counting_type: u8,
        full_timestamp: bool,
        discontinuity: bool,
        cnt_dropped: bool,
        n_frames: u16,
        seconds_value: Option<u8>,
        minutes_value: Option<u8>,
        hours_value: Option<u8>,
        time_offset_length: usize,
        time_offset_value: Option<u32>,
    },
}

impl Metadata {
    pub fn decode(buf: &mut Buffer) -> Result<Self, ObuError> {
        // metadata_type	leb128()
        let kind = buf.get_leb128() as u8;
        Ok(match MetadataType::try_from(kind)? {
            MetadataType::UnregisteredUserPrivate => Self::UnregisteredUserPrivate(kind),
            MetadataType::ItutT35 => {
                // itu_t_t35_country_code	f(8)
                let country_code = buf.get_bits(8) as u8;
                let country_code_extension_byte = if country_code == 0xFF {
                    // itu_t_t35_country_code_extension_byte	f(8)
                    Some(buf.get_bits(8) as u8)
                } else {
                    None
                };

                Self::ItutT35 {
                    country_code,
                    country_code_extension_byte,
                }
            }
            MetadataType::HdrCll => {
                Self::HdrCll {
                    // max_cll	f(16)
                    max_cll: buf.get_bits(16) as u16,
                    // max_fall	f(16)
                    max_fall: buf.get_bits(16) as u16,
                }
            }
            MetadataType::HdrMdcv => {
                let mut primary_chromaticity_x = [0u16; 3];
                let mut primary_chromaticity_y = [0u16; 3];
                for i in 0..3 {
                    // primary_chromaticity_x[ i ]	f(16)
                    primary_chromaticity_x[i] = buf.get_bits(16) as u16;

                    // primary_chromaticity_y[ i ]	f(16)
                    primary_chromaticity_y[i] = buf.get_bits(16) as u16;
                }

                Self::HdrMdcv {
                    primary_chromaticity_x,
                    primary_chromaticity_y,
                    // white_point_chromaticity_x	f(16)
                    white_point_chromaticity_x: buf.get_bits(16) as u16,
                    // white_point_chromaticity_y	f(16)
                    white_point_chromaticity_y: buf.get_bits(16) as u16,
                    // luminance_max	f(32)
                    luminance_max: buf.get_bits(32),
                    // luminance_min	f(32)
                    luminance_min: buf.get_bits(32),
                }
            }
            MetadataType::Scalability => {
                // scalability_mode_idc	f(8)
                let mode_idc = ScalabilityModeIdc::try_from(buf.get_bits(8) as u8)?;
                let scalability_structure = if mode_idc == ScalabilityModeIdc::SS {
                    Some(ScalabilityStructure::decode(buf))
                } else {
                    None
                };

                Self::Scalability {
                    mode_idc,
                    scalability_structure,
                }
            }
            MetadataType::Timecode => {
                // counting_type	f(5)
                let counting_type = buf.get_bits(5) as u8;

                // full_timestamp_flag	f(1)
                let full_timestamp = buf.get_bit();

                // discontinuity_flag	f(1)
                let discontinuity = buf.get_bit();

                // cnt_dropped_flag	f(1)
                let cnt_dropped = buf.get_bit();

                // n_frames	f(9)
                let n_frames = buf.get_bits(9) as u16;

                let mut seconds_value = None;
                let mut minutes_value = None;
                let mut hours_value = None;
                if full_timestamp {
                    // seconds_value	f(6)
                    seconds_value = Some(buf.get_bits(6) as u8);

                    // minutes_value	f(6)
                    minutes_value = Some(buf.get_bits(6) as u8);

                    // hours_value	f(5)
                    hours_value = Some(buf.get_bits(6) as u8);
                } else {
                    // seconds_flag	f(1)
                    if buf.get_bit() {
                        // seconds_value	f(6)
                        seconds_value = Some(buf.get_bits(6) as u8);

                        // minutes_flag	f(1)
                        if buf.get_bit() {
                            // minutes_value	f(6)
                            minutes_value = Some(buf.get_bits(6) as u8);

                            // hours_flag	f(1)
                            if buf.get_bit() {
                                // hours_value	f(5)
                                hours_value = Some(buf.get_bits(6) as u8);
                            }
                        }
                    }
                }

                // time_offset_length	f(5)
                let time_offset_length = buf.get_bits(5) as usize;
                let time_offset_value = if time_offset_length > 0 {
                    Some(buf.get_bits(time_offset_length))
                } else {
                    None
                };

                Self::Timecode {
                    counting_type,
                    full_timestamp,
                    discontinuity,
                    cnt_dropped,
                    n_frames,
                    seconds_value,
                    minutes_value,
                    hours_value,
                    time_offset_length,
                    time_offset_value,
                }
            }
        })
    }
}
