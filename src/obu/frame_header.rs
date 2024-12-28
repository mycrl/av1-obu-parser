use super::{
    sequence_header::FrameIdNumbersPresent, Buffer, ObuContext, ObuError, ObuUnknownError,
};

use crate::constants::{
    NUM_REF_FRAMES, PRIMARY_REF_NONE, REFS_PER_FRAME, SELECT_INTEGER_MV,
    SELECT_SCREEN_CONTENT_TOOLS, SUPERRES_DENOM_BITS, SUPERRES_DENOM_MIN, SUPERRES_NUM,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    KeyFrame,
    InterFrame,
    InterOnlyFrame,
    SwitchFrame,
}

impl TryFrom<u8> for FrameType {
    type Error = ObuError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::KeyFrame,
            1 => Self::InterFrame,
            2 => Self::InterOnlyFrame,
            3 => Self::SwitchFrame,
            _ => return Err(ObuError::Unknown(ObuUnknownError::FrameType)),
        })
    }
}

#[derive(Debug, Clone)]
pub struct TemporalPointInfo {
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

#[inline]
pub fn compute_image_size(ctx: &mut ObuContext) {
    ctx.mi_cols = 2 * ((ctx.frame_width + 7) >> 3) as u32;
    ctx.mi_rows = 2 * ((ctx.frame_height + 7) >> 3) as u32;
}

#[inline]
pub fn frame_size(ctx: &mut ObuContext, frame_size_override: bool, buf: &mut Buffer) {
    let sequence_header = ctx
        .sequence_header
        .as_ref()
        .expect("sequence header cannot be found, this is a undefined behavior!");

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

#[inline]
pub fn superres_params(ctx: &mut ObuContext, buf: &mut Buffer) {
    let sequence_header = ctx
        .sequence_header
        .as_ref()
        .expect("sequence header cannot be found, this is a undefined behavior!");

    let use_superres = if sequence_header.enable_superres {
        // use_superres	f(1)
        buf.get_bit()
    } else {
        false
    };

    ctx.superres_denom = if use_superres {
        // coded_denom	f(SUPERRES_DENOM_BITS)
        let coded_denom = buf.get_bits(SUPERRES_DENOM_BITS as usize) as u8;
        coded_denom + SUPERRES_DENOM_MIN
    } else {
        SUPERRES_NUM
    };

    ctx.upscaled_width = ctx.frame_width;
    ctx.frame_width = (ctx.upscaled_width * SUPERRES_NUM as u16 + (ctx.superres_denom as u16 / 2))
        / ctx.superres_denom as u16;
}

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

#[inline]
pub fn frame_size_with_refs(ctx: &mut ObuContext, frame_size_override: bool, buf: &mut Buffer) {
    let mut found_ref = false;
    for _ in 0..REFS_PER_FRAME {
        // found_ref	f(1)
        found_ref = buf.get_bit();

        // if ( found_ref == 1 ) {
        //     UpscaledWidth = RefUpscaledWidth[ ref_frame_idx[ i ] ]
        //     FrameWidth = UpscaledWidth
        //     FrameHeight = RefFrameHeight[ ref_frame_idx[ i ] ]
        //     RenderWidth = RefRenderWidth[ ref_frame_idx[ i ] ]
        //     RenderHeight = RefRenderHeight[ ref_frame_idx[ i ] ]
        //     break
        // }
    }

    if !found_ref {
        frame_size(ctx, frame_size_override, buf);
        render_size(ctx, buf);
    } else {
        superres_params(ctx, buf);
        compute_image_size(ctx);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterpolationFilter {
    Eighttap,
    EighttapSmooth,
    EighttapSharp,
    Bilinear,
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

#[derive(Debug, Clone)]
pub struct UncompressedHeader;

impl UncompressedHeader {
    pub fn decode(ctx: &mut ObuContext, buf: &mut Buffer) -> Result<Self, ObuError> {
        let sequence_header = ctx
            .sequence_header
            .as_ref()
            .ok_or_else(|| ObuError::NotFoundSequenceHeader)?;

        let mut id_len = 0;
        if let Some(ref value) = sequence_header.frame_id_numbers_present {
            id_len = value.additional_frame_id_length as usize
                + value.delta_frame_id_length as usize
                + 3;
        }

        let all_frames = (1 << NUM_REF_FRAMES) - 1;

        let mut show_existing_frame = false;
        let mut frame_type = FrameType::KeyFrame;
        let mut show_frame = true;
        let mut showable_frame = false;

        let mut refresh_frame_flags = 0;
        let mut error_resilient_mode = false;
        let mut temporal_point_info = None;
        let mut display_frame_id = None;

        if sequence_header.reduced_still_picture_header {
            ctx.frame_is_intra = true;
        } else {
            // show_existing_frame	f(1)
            show_existing_frame = buf.get_bit();

            if show_existing_frame {
                // frame_to_show_map_idx	f(3)
                let frame_to_show_map_idx = buf.get_bits(3) as u8;
                if let Some(decoder_model_info) = &sequence_header.decoder_model_info {
                    if !sequence_header
                        .timing_info
                        .map(|v| v.equal_picture_interval.is_some())
                        .unwrap_or(false)
                    {
                        temporal_point_info = Some(TemporalPointInfo::decode(
                            buf.as_mut(),
                            decoder_model_info.frame_presentation_time_length as usize,
                        ));
                    }
                }

                if sequence_header.frame_id_numbers_present.is_some() {
                    // display_frame_id	f(idLen)
                    display_frame_id = Some(buf.get_bits(id_len));
                }

                frame_type = *ctx
                    .frame_type_refs
                    .get(frame_to_show_map_idx as usize)
                    .ok_or_else(|| ObuError::Unknown(ObuUnknownError::FrameTypeRefIndex))?;

                if frame_type == FrameType::KeyFrame {
                    refresh_frame_flags = all_frames;
                }

                if sequence_header.film_grain_params_present {
                    // TODO
                    // load_grain_params( frame_to_show_map_idx )
                }

                // TODO
                // return
            }

            // frame_type	f(2)
            frame_type = FrameType::try_from(buf.get_bits(2) as u8)?;
            ctx.frame_is_intra =
                frame_type == FrameType::InterOnlyFrame || frame_type == FrameType::KeyFrame;

            // show_frame	f(1)
            show_frame = buf.get_bit();

            if show_frame {
                if let Some(decoder_model_info) = &sequence_header.decoder_model_info {
                    if !sequence_header
                        .timing_info
                        .map(|v| v.equal_picture_interval.is_some())
                        .unwrap_or(false)
                    {
                        let temporal_point_info = TemporalPointInfo::decode(
                            buf.as_mut(),
                            decoder_model_info.frame_presentation_time_length as usize,
                        );
                    }
                }

                showable_frame = frame_type != FrameType::KeyFrame;
            } else {
                // showable_frame	f(1)
                showable_frame = buf.get_bit();
            }

            error_resilient_mode = if frame_type == FrameType::SwitchFrame
                || frame_type == FrameType::KeyFrame && show_frame
            {
                true
            } else {
                // error_resilient_mode	f(1)
                buf.get_bit()
            };
        }

        // TODO
        // if ( frame_type == KEY_FRAME && show_frame ) {
        //     for ( i = 0; i < NUM_REF_FRAMES; i++ ) {
        //         RefValid[ i ] = 0
        //         RefOrderHint[ i ] = 0
        //     }
        //     for ( i = 0; i < REFS_PER_FRAME; i++ ) {
        //         OrderHints[ LAST_FRAME + i ] = 0
        //     }
        // }

        // disable_cdf_update	f(1)
        let disable_cdf_update = buf.get_bit();
        let allow_screen_content_tools =
            if sequence_header.seq_force_screen_content_tools == SELECT_SCREEN_CONTENT_TOOLS {
                // allow_screen_content_tools	f(1)
                buf.get_bit()
            } else {
                sequence_header.seq_force_screen_content_tools != 0
            };

        let mut force_integer_mv = if allow_screen_content_tools {
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

        let current_frame_id = if let Some(f) = &sequence_header.frame_id_numbers_present {
            // current_frame_id	f(idLen)
            buf.get_bits(id_len)

            // TODO
            // mark_ref_frames( idLen )
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

        let mut buffer_removal_times = Vec::with_capacity(sequence_header.operating_points.len());
        if let Some(decoder_model_info) = sequence_header.decoder_model_info {
            // buffer_removal_time_present_flag	f(1)
            let buffer_removal_time_present_flag = buf.get_bit();
            if buffer_removal_time_present_flag {
                for operating_point in &sequence_header.operating_points {
                    if operating_point.operating_parameters_info.is_some() {
                        if let Some(extension) = ctx.obu_header_extension {
                            let op_pt_dic = operating_point.idc;
                            let in_temporal_layer = ((op_pt_dic >> extension.temporal_id) & 1) != 0;
                            let in_spatial_layer =
                                ((op_pt_dic >> (extension.spatial_id + 8)) & 1) != 0;
                            if op_pt_dic == 0 || (in_temporal_layer && in_spatial_layer) {
                                // buffer_removal_time[ opNum ]	f(n)
                                buffer_removal_times.push(buf.get_bits(
                                    decoder_model_info.buffer_removal_time_length as usize,
                                ));
                            }
                        }
                    }
                }
            }
        }

        let mut allow_high_precision_mv = false;
        let mut use_ref_frame_mvs = false;
        let mut allow_intrabc = false;

        refresh_frame_flags = if frame_type == FrameType::SwitchFrame
            || frame_type == FrameType::KeyFrame && show_frame
        {
            all_frames
        } else {
            buf.get_bits(8)
        };

        let mut ref_order_hints = None;
        if (!ctx.frame_is_intra || refresh_frame_flags != all_frames)
            && error_resilient_mode
            && sequence_header.enable_order_hint
        {
            let mut hints = [0u32; NUM_REF_FRAMES as usize];
            for i in 0..NUM_REF_FRAMES as usize {
                // ref_order_hint[ i ]	f(OrderHintBits)
                hints[i] = buf.get_bits(ctx.order_hint_bits);
            }

            ref_order_hints = Some(hints);
        }

        let mut allow_intrabc = false;
        if ctx.frame_is_intra {
            frame_size(ctx, frame_size_override, buf);
            render_size(ctx, buf);
            if allow_screen_content_tools && ctx.upscaled_width == ctx.frame_width {
                // allow_intrabc	f(1)
                allow_intrabc = buf.get_bit();
            }
        } else {
            let mut frame_refs_short_signaling = false;
            if !sequence_header.enable_order_hint {
                // frame_refs_short_signaling	f(1)
                frame_refs_short_signaling = buf.get_bit();
                if frame_refs_short_signaling {
                    // last_frame_idx	f(3)
                    let last_frame_idx = buf.get_bits(3);

                    // gold_frame_idx	f(3)
                    let gold_frame_idx = buf.get_bits(3);

                    // TODO
                    // set_frame_refs()
                }
            }

            for _ in 0..REFS_PER_FRAME {
                if frame_refs_short_signaling {
                    // ref_frame_idx[ i ]	f(3)
                    let ref_frame_idx = buf.get_bits(3);
                }

                if let Some(frame_id_numbers_present) = &sequence_header.frame_id_numbers_present {
                    let n = frame_id_numbers_present.delta_frame_id_length;
                    // delta_frame_id_minus_1	f(n)
                    let delta_frame_id = buf.get_bits(n as usize) + 1;
                    ctx.delta_frame_id = delta_frame_id;

                    // expectedFrameId[ i ] = ((current_frame_id + (1 << idLen)
                    // - DeltaFrameId ) % (1 << idLen))
                }
            }

            if frame_size_override && !error_resilient_mode {
                frame_size_with_refs(ctx, frame_size_override, buf);
            } else {
                frame_size(ctx, frame_size_override, buf);
                render_size(ctx, buf);
            }

            let allow_high_precision_mv = if force_integer_mv {
                false
            } else {
                // allow_high_precision_mv	f(1)
                buf.get_bit()
            };

            let interpolation_filter = read_interpolation_filter(buf)?;

            // is_motion_mode_switchable	f(1)
            let is_motion_mode_switchable = buf.get_bit();
            use_ref_frame_mvs = if error_resilient_mode || !sequence_header.enable_ref_frame_mvs {
                false
            } else {
                // use_ref_frame_mvs	f(1)
                buf.get_bit()
            };

            // for ( i = 0; i < REFS_PER_FRAME; i++ ) {
            //     refFrame = LAST_FRAME + i
            //     hint = RefOrderHint[ ref_frame_idx[ i ] ]
            //     OrderHints[ refFrame ] = hint
            //     if ( !enable_order_hint ) {
            //         RefFrameSignBias[ refFrame ] = 0
            //     } else {
            //         RefFrameSignBias[ refFrame ] = get_relative_dist( hint, OrderHint) > 0
            //     }
            // }
        }

        let disable_frame_end_update_cdf =
            if sequence_header.reduced_still_picture_header || disable_cdf_update {
                true
            } else {
                // disable_frame_end_update_cdf	f(1)
                buf.get_bit()
            };

        if primary_ref_frame == PRIMARY_REF_NONE {
        } else {
        }

        Ok(Self {})
    }
}

#[derive(Debug, Clone)]
pub struct FrameHeader {}

impl FrameHeader {
    pub fn decode(ctx: &mut ObuContext, buf: &mut Buffer) -> Result<Self, ObuError> {
        if ctx.seen_frame_header {
            // TODO:
            // frame_header_copy
        } else {
            ctx.seen_frame_header = true;
        }

        Ok(Self {})
    }
}
