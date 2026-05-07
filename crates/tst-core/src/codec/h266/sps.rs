//! H.266 SPS parser. Per H.266 V4 §7.3.2.4.

use crate::codec::h265::bitreader::BitReader;
use crate::codec::h266::profile_tier_level::{H266ProfileTierLevel, parse_into};
use crate::codec::h266::vui::parse_h266_vui;
use crate::codec::{ChromaFormat, ColorInfo, ParseError, Rational, validate_bit_depth_minus8};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H266Sps {
    pub sps_id: u8,
    pub vps_id: u8,
    pub profile_tier_level: H266ProfileTierLevel,
    pub width: u32,
    pub height: u32,
    pub chroma_format: ChromaFormat,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub color_info: Option<ColorInfo>,
    pub frame_rate: Option<Rational>,
    /// Luma-sample conformance-window crop offsets per H.266 §7.4.3.4.
    /// `coded_width = width + crop_left + crop_right` (and similarly for
    /// height). Useful for sizing GPU buffers.
    pub crop_left: u32,
    pub crop_right: u32,
    pub crop_top: u32,
    pub crop_bottom: u32,
    pub raw_rbsp: Vec<u8>,
}

impl H266Sps {
    /// Pre-crop luma width — the value of `pic_width_max_in_luma_samples`
    /// before conformance-window cropping was applied.
    pub fn coded_width(&self) -> u32 {
        self.width + self.crop_left + self.crop_right
    }
    /// Pre-crop luma height — the value of `pic_height_max_in_luma_samples`
    /// before conformance-window cropping was applied.
    pub fn coded_height(&self) -> u32 {
        self.height + self.crop_top + self.crop_bottom
    }
}

/// Parse an H.266 SPS RBSP (Annex-B start codes already stripped,
/// emulation-prevention bytes preserved). Per H.266 V4 §7.3.2.4.
///
/// v0 scope surfaces: `sps_id`, `vps_id`, headline `profile_tier_level`
/// fields, dimensions, chroma format, bit depth, and (optionally)
/// `color_info` + `frame_rate` from VUI. Conformance-window cropping is
/// applied to width/height before they are returned.
///
/// Bails `ParseError::UnsupportedProfile` on `sps_subpic_info_present_flag`
/// and `sps_scaling_list_data_present_flag` paths — these are rare in
/// reference-encoder defaults and would require walking large per-tile
/// or per-coefficient blocks not modeled here. Same conservative stance
/// as `codec::h265::parse_sps`.
pub fn parse_sps(rbsp: &[u8]) -> Result<H266Sps, ParseError> {
    if rbsp.is_empty() {
        return Err(ParseError::TruncatedRbsp {
            offset_bits: 0,
            needed_bits: 8,
        });
    }
    let mut br = BitReader::new(rbsp);

    // §7.3.2.4 SPS header.
    let sps_id = br.read_u(4)? as u8;
    let vps_id = br.read_u(4)? as u8;
    let max_sublayers_minus1 = br.read_u(3)? as u8;
    let chroma_format_idc = br.read_u(2)?;
    let log2_ctu_size_minus5 = br.read_u(2)?;
    let ptl_dpb_hrd_present = br.read_bool()?;

    // profile_tier_level(1, sps_max_sublayers_minus1) when flag is set.
    // The PTL walks past its headline fields — alignment matters for
    // every subsequent ue(v) read, so the full PTL syntax must be
    // consumed (not just the 16-bit headline that the standalone
    // `parse_profile_tier_level` exposes).
    let mut profile_tier_level = H266ProfileTierLevel::default();
    if ptl_dpb_hrd_present {
        parse_into(&mut br, true, max_sublayers_minus1, &mut profile_tier_level)?;
    }

    let _gdr_enabled_flag = br.read_bool()?;
    let ref_pic_resampling_enabled_flag = br.read_bool()?;
    if ref_pic_resampling_enabled_flag {
        let _res_change_in_clvs_allowed_flag = br.read_bool()?;
    }

    let pic_width_max_in_luma_samples = br.read_ue()?;
    let pic_height_max_in_luma_samples = br.read_ue()?;

    // Conformance-window cropping (semantics match H.265 §7.4.3.2.1).
    let conformance_window_flag = br.read_bool()?;
    let (mut crop_x_left, mut crop_x_right, mut crop_y_top, mut crop_y_bottom) = (0u32, 0, 0, 0);
    if conformance_window_flag {
        let conf_win_left_offset = br.read_ue()?;
        let conf_win_right_offset = br.read_ue()?;
        let conf_win_top_offset = br.read_ue()?;
        let conf_win_bottom_offset = br.read_ue()?;
        let (sub_w, sub_h) = match chroma_format_idc {
            1 => (2u32, 2u32),
            2 => (2, 1),
            3 => (1, 1),
            _ => (1, 1),
        };
        crop_x_left = sub_w * conf_win_left_offset;
        crop_x_right = sub_w * conf_win_right_offset;
        crop_y_top = sub_h * conf_win_top_offset;
        crop_y_bottom = sub_h * conf_win_bottom_offset;
    }

    let subpic_info_present_flag = br.read_bool()?;
    if subpic_info_present_flag {
        // The subpic block reads variable-width per-subpic fields whose
        // bit-widths depend on CTU size and picture dimensions.
        // Modeling that path adds significant complexity for streams
        // that almost never occur in non-multi-picture encoders.
        return Err(ParseError::UnsupportedProfile {
            profile_idc: profile_tier_level.general_profile_idc,
        });
    }

    let bit_depth_minus8 = br.read_ue()?;
    let bit_depth_luma = validate_bit_depth_minus8("sps_bitdepth_minus8", bit_depth_minus8)?;
    // H.266 §7.4.3.4 has a single `sps_bitdepth_minus8` covering both
    // luma and chroma — spec invariant, not a parser simplification.
    let bit_depth_chroma = bit_depth_luma;

    // The chroma format derives directly from sps_chroma_format_idc.
    let chroma_format = match chroma_format_idc {
        0 => ChromaFormat::Monochrome,
        1 => ChromaFormat::Yuv420,
        2 => ChromaFormat::Yuv422,
        3 => ChromaFormat::Yuv444,
        other => {
            return Err(ParseError::ReservedValue {
                field: "sps_chroma_format_idc",
                value: other,
            });
        }
    };

    // Width/height after conformance-window cropping.
    let width = pic_width_max_in_luma_samples.saturating_sub(crop_x_left + crop_x_right);
    let height = pic_height_max_in_luma_samples.saturating_sub(crop_y_top + crop_y_bottom);

    // SPS body walk per H.266 V4 §7.3.2.4 — entropy_coding_sync,
    // log2_max_pic_order_cnt, partition constraints, ... up to the VUI
    // flag. Bails UnsupportedProfile on subpic / scaling list paths
    // (rare in default-encoder output).
    let (color_info, frame_rate) = walk_sps_body(
        &mut br,
        ptl_dpb_hrd_present,
        max_sublayers_minus1,
        vps_id,
        chroma_format_idc,
        log2_ctu_size_minus5,
    )?;

    Ok(H266Sps {
        sps_id,
        vps_id,
        profile_tier_level,
        width,
        height,
        chroma_format,
        bit_depth_luma,
        bit_depth_chroma,
        color_info,
        frame_rate,
        crop_left: crop_x_left,
        crop_right: crop_x_right,
        crop_top: crop_y_top,
        crop_bottom: crop_y_bottom,
        raw_rbsp: rbsp.to_vec(),
    })
}

// ─── SPS body walker ──────────────────────────────────────────────────────────

/// Walk the SPS RBSP body from `sps_entropy_coding_sync_enabled_flag` through
/// `sps_vui_parameters_present_flag` (§7.3.2.4), then call the VUI parser if
/// the flag is set.
///
/// Returns `(color_info, frame_rate)`. Both stay `None` until Task 4.3 wires
/// the VUI walker; this function just lands the bit cursor correctly.
fn walk_sps_body(
    br: &mut BitReader<'_>,
    ptl_dpb_hrd_present: bool,
    max_sublayers_minus1: u8,
    vps_id: u8,
    chroma_format_idc: u32,
    log2_ctu_size_minus5: u32,
) -> Result<(Option<ColorInfo>, Option<Rational>), ParseError> {
    // §7.3.2.4 — entropy coding / entry-point config.
    let _entropy_coding_sync_enabled_flag = br.read_bool()?;
    let _entry_point_offsets_present_flag = br.read_bool()?;

    // §7.3.2.4 — POC config.
    let _log2_max_pic_order_cnt_lsb_minus4 = br.read_u(4)?; // u(4)
    let poc_msb_cycle_flag = br.read_bool()?;
    if poc_msb_cycle_flag {
        let _poc_msb_cycle_len_minus1 = br.read_ue()?;
    }

    // §7.3.2.4 — extra PH/SH bytes: each byte contributes 8 flag bits.
    let sps_num_extra_ph_bytes = br.read_u(2)?; // u(2)
    for _ in 0..(sps_num_extra_ph_bytes * 8) {
        br.read_bool()?; // sps_extra_ph_bit_present_flag[i]
    }
    let sps_num_extra_sh_bytes = br.read_u(2)?; // u(2)
    for _ in 0..(sps_num_extra_sh_bytes * 8) {
        br.read_bool()?; // sps_extra_sh_bit_present_flag[i]
    }

    // §7.3.2.4 — DPB parameters (conditional on PTL/DPB/HRD present flag).
    // Per §7.3.4, dpb_parameters(MaxSubLayersMinus1, subLayerInfoFlag) loops
    // from (subLayerInfoFlag ? 0 : MaxSubLayersMinus1) to MaxSubLayersMinus1.
    if ptl_dpb_hrd_present {
        let sublayer_dpb_params_flag = if max_sublayers_minus1 > 0 {
            br.read_bool()? // sps_sublayer_dpb_params_flag
        } else {
            false
        };
        walk_dpb_parameters(br, max_sublayers_minus1, sublayer_dpb_params_flag)?;
    }

    // §7.3.2.4 — minimum luma coding-block size (ue(v)).
    let _log2_min_luma_coding_block_size_minus2 = br.read_ue()?;

    // CtbSizeY = 1 << (sps_log2_ctu_size_minus5 + 5).
    let ctu_size_y = 1u32 << (log2_ctu_size_minus5 + 5);

    // §7.3.2.4 — partition constraints block.
    let partition_constraints_override_enabled_flag = br.read_bool()?;
    let _ = partition_constraints_override_enabled_flag; // read for cursor only

    // Intra luma QT/MTT partition constraints (always present).
    let _log2_diff_min_qt_min_cb_intra_luma = br.read_ue()?;
    let max_mtt_depth_intra_luma = br.read_ue()?;
    if max_mtt_depth_intra_luma != 0 {
        let _log2_diff_max_bt_min_qt_intra_luma = br.read_ue()?;
        let _log2_diff_max_tt_min_qt_intra_luma = br.read_ue()?;
    }

    // Dual-tree intra chroma (only present when chroma_format_idc != 0).
    let qtbtt_dual_tree_intra_flag = if chroma_format_idc != 0 {
        br.read_bool()? // sps_qtbtt_dual_tree_intra_flag
    } else {
        false
    };
    if qtbtt_dual_tree_intra_flag {
        let _log2_diff_min_qt_min_cb_intra_chroma = br.read_ue()?;
        let max_mtt_depth_intra_chroma = br.read_ue()?;
        if max_mtt_depth_intra_chroma != 0 {
            let _log2_diff_max_bt_min_qt_intra_chroma = br.read_ue()?;
            let _log2_diff_max_tt_min_qt_intra_chroma = br.read_ue()?;
        }
    }

    // Inter partition constraints (always present).
    let _log2_diff_min_qt_min_cb_inter = br.read_ue()?;
    let max_mtt_depth_inter = br.read_ue()?;
    if max_mtt_depth_inter != 0 {
        let _log2_diff_max_bt_min_qt_inter = br.read_ue()?;
        let _log2_diff_max_tt_min_qt_inter = br.read_ue()?;
    }

    // §7.3.2.4 — max luma transform size flag (present only when CTBsize > 32).
    let max_luma_transform_size_64_flag = if ctu_size_y > 32 {
        br.read_bool()? // sps_max_luma_transform_size_64_flag
    } else {
        false
    };

    // §7.3.2.4 — transform / coding tool flags.
    let transform_skip_enabled_flag = br.read_bool()?;
    if transform_skip_enabled_flag {
        let _log2_transform_skip_max_size_minus2 = br.read_ue()?;
        let _bdpcm_enabled_flag = br.read_bool()?;
    }
    let mts_enabled_flag = br.read_bool()?;
    if mts_enabled_flag {
        let _explicit_mts_intra_enabled_flag = br.read_bool()?;
        let _explicit_mts_inter_enabled_flag = br.read_bool()?;
    }
    let lfnst_enabled_flag = br.read_bool()?;

    // §7.3.2.4 — chroma QP table (present when chroma_format_idc != 0).
    let _joint_cbcr_enabled_flag = if chroma_format_idc != 0 {
        let joint_cbcr = br.read_bool()?; // sps_joint_cbcr_enabled_flag
        let same_qp_table_for_chroma_flag = br.read_bool()?;
        // numQpTables depends on same_qp_table and joint_cbcr:
        //   same_qp_table → 1; else joint_cbcr → 3; else → 2.
        let num_qp_tables = if same_qp_table_for_chroma_flag {
            1
        } else if joint_cbcr {
            3
        } else {
            2
        };
        for _ in 0..num_qp_tables {
            let _qp_table_start_minus26 = br.read_se()?;
            let num_points = br.read_ue()?; // sps_num_points_in_qp_table_minus1
            for _ in 0..=num_points {
                let _delta_qp_in_val_minus1 = br.read_ue()?;
                let _delta_qp_diff_val = br.read_ue()?;
            }
        }
        joint_cbcr
    } else {
        false
    };

    // §7.3.2.4 — filter / tool presence flags.
    let _sao_enabled_flag = br.read_bool()?;
    let alf_enabled_flag = br.read_bool()?;
    if alf_enabled_flag && chroma_format_idc != 0 {
        let _ccalf_enabled_flag = br.read_bool()?;
    }
    let _lmcs_enabled_flag = br.read_bool()?;
    let _weighted_pred_flag = br.read_bool()?;
    let _weighted_bipred_flag = br.read_bool()?;
    let long_term_ref_pics_flag = br.read_bool()?;
    // sps_inter_layer_prediction_enabled_flag present only when vps_id > 0.
    let inter_layer_pred_enabled_flag = if vps_id > 0 { br.read_bool()? } else { false };

    // §7.3.2.4 — reference picture list config.
    let _idr_rpl_present_flag = br.read_bool()?;
    let rpl1_same_as_rpl0_flag = br.read_bool()?;
    // Loop over both RPL directions (or just L0 if rpl1_same_as_rpl0).
    let num_directions = if rpl1_same_as_rpl0_flag { 1u32 } else { 2u32 };
    for list_idx in 0..num_directions {
        let num_ref_pic_lists = br.read_ue()?; // sps_num_ref_pic_lists[i]
        for rpls_idx in 0..num_ref_pic_lists {
            walk_ref_pic_list_struct(
                br,
                list_idx,
                rpls_idx,
                num_ref_pic_lists,
                long_term_ref_pics_flag,
                inter_layer_pred_enabled_flag,
            )?;
        }
    }

    // §7.3.2.4 — motion-prediction tool flags.
    let _ref_wraparound_enabled_flag = br.read_bool()?;
    let temporal_mvp_enabled_flag = br.read_bool()?;
    if temporal_mvp_enabled_flag {
        let _sbtmvp_enabled_flag = br.read_bool()?;
    }
    let amvr_enabled_flag = br.read_bool()?;
    let bdof_enabled_flag = br.read_bool()?;
    if bdof_enabled_flag {
        let _bdof_control_present_in_ph_flag = br.read_bool()?;
    }
    let _smvd_enabled_flag = br.read_bool()?;
    let dmvr_enabled_flag = br.read_bool()?;
    if dmvr_enabled_flag {
        let _dmvr_control_present_in_ph_flag = br.read_bool()?;
    }
    let mmvd_enabled_flag = br.read_bool()?;
    if mmvd_enabled_flag {
        let _mmvd_fullpel_only_enabled_flag = br.read_bool()?;
    }

    // sps_six_minus_max_num_merge_cand encodes MaxNumMergeCand = 6 - value.
    let six_minus_max_num_merge_cand = br.read_ue()?;
    let max_num_merge_cand = 6u32.saturating_sub(six_minus_max_num_merge_cand);

    let _sbt_enabled_flag = br.read_bool()?;
    let affine_enabled_flag = br.read_bool()?;
    if affine_enabled_flag {
        let _five_minus_max_num_subblock_merge_cand = br.read_ue()?;
        let _six_param_affine_enabled_flag = br.read_bool()?;
        if amvr_enabled_flag {
            let _affine_amvr_enabled_flag = br.read_bool()?;
        }
        let affine_prof_enabled_flag = br.read_bool()?;
        if affine_prof_enabled_flag {
            let _prof_control_present_in_ph_flag = br.read_bool()?;
        }
    }
    let _bcw_enabled_flag = br.read_bool()?;
    let _ciip_enabled_flag = br.read_bool()?;

    if max_num_merge_cand >= 2 {
        let gpm_enabled_flag = br.read_bool()?;
        if gpm_enabled_flag && max_num_merge_cand >= 3 {
            let _max_num_merge_cand_minus_max_num_gpm_cand = br.read_ue()?;
        }
    }

    let _log2_parallel_merge_level_minus2 = br.read_ue()?;
    let _isp_enabled_flag = br.read_bool()?;
    let _mrl_enabled_flag = br.read_bool()?;
    let _mip_enabled_flag = br.read_bool()?;
    if chroma_format_idc != 0 {
        let _cclm_enabled_flag = br.read_bool()?;
    }
    if chroma_format_idc == 1 {
        let _chroma_horizontal_collocated_flag = br.read_bool()?;
        let _chroma_vertical_collocated_flag = br.read_bool()?;
    }

    let palette_enabled_flag = br.read_bool()?;
    let act_enabled_flag = if chroma_format_idc == 3 && !max_luma_transform_size_64_flag {
        br.read_bool()? // sps_act_enabled_flag
    } else {
        false
    };
    if transform_skip_enabled_flag || palette_enabled_flag {
        let _sps_min_qp_prime_ts = br.read_ue()?;
    }

    let ibc_enabled_flag = br.read_bool()?;
    if ibc_enabled_flag {
        let _six_minus_max_num_ibc_merge_cand = br.read_ue()?;
    }

    let ladf_enabled_flag = br.read_bool()?;
    if ladf_enabled_flag {
        let num_ladf_intervals_minus2 = br.read_u(2)?; // u(2)
        let _ladf_lowest_interval_qp_offset = br.read_se()?;
        for _ in 0..(num_ladf_intervals_minus2 + 1) {
            let _ladf_qp_offset = br.read_se()?;
            let _ladf_delta_threshold_minus1 = br.read_ue()?;
        }
    }

    // §7.3.2.4 — scaling list. Bail on explicit scaling lists — walking
    // scaling_list_data() is complex and rarely enabled in reference encoders.
    let sps_explicit_scaling_list_enabled_flag = br.read_bool()?;
    if sps_explicit_scaling_list_enabled_flag {
        return Err(ParseError::UnsupportedProfile {
            profile_idc: 0, // profile_idc not relevant here; bail is structural
        });
    }
    // These two flags are conditional on lfnst/act enabled AND explicit
    // scaling list enabled — both false since we bailed above if the latter
    // was set; but read the guard conditions correctly for any future path.
    if lfnst_enabled_flag && sps_explicit_scaling_list_enabled_flag {
        let _scaling_matrix_for_lfnst_disabled_flag = br.read_bool()?;
    }
    if act_enabled_flag && sps_explicit_scaling_list_enabled_flag {
        let _scaling_matrix_for_alternative_colour_space_disabled_flag = br.read_bool()?;
        // sps_scaling_matrix_designated_colour_space_flag is gated on the
        // flag just read, which we can't know here (already bailed before
        // it becomes relevant in the flag-true path).
    }

    let _dep_quant_enabled_flag = br.read_bool()?;
    let _sign_data_hiding_enabled_flag = br.read_bool()?;

    // §7.3.2.4 — virtual boundaries (per-picture or per-SPS signalling).
    let virtual_boundaries_enabled_flag = br.read_bool()?;
    if virtual_boundaries_enabled_flag {
        let virtual_boundaries_present_in_sps_flag = br.read_bool()?;
        if virtual_boundaries_present_in_sps_flag {
            let num_ver_virtual_boundaries = br.read_ue()?;
            for _ in 0..num_ver_virtual_boundaries {
                let _sps_virtual_boundary_pos_x_minus1 = br.read_ue()?;
            }
            let num_hor_virtual_boundaries = br.read_ue()?;
            for _ in 0..num_hor_virtual_boundaries {
                let _sps_virtual_boundary_pos_y_minus1 = br.read_ue()?;
            }
        }
    }

    // §7.3.2.4 — timing/HRD parameters (conditional on PTL/DPB/HRD flag).
    // Per §7.3.5.1, general_timing_hrd_parameters() contains num_units_in_tick
    // + time_scale. Task 4.3 will capture these for frame_rate; here we read
    // and discard them to keep the cursor positioned correctly for VUI.
    if ptl_dpb_hrd_present {
        let timing_hrd_params_present_flag = br.read_bool()?;
        if timing_hrd_params_present_flag {
            walk_timing_hrd_parameters(br, max_sublayers_minus1)?;
        }
    }

    let _sps_field_seq_flag = br.read_bool()?;

    // §7.3.2.4 — VUI parameters.
    let sps_vui_parameters_present_flag = br.read_bool()?;
    let (color_info, frame_rate) = if sps_vui_parameters_present_flag {
        let vui_payload_size_minus1 = br.read_ue()? as usize;
        // §7.3.2.4: "while( !byte_aligned() ) sps_vui_alignment_zero_bit f(1)"
        // Consume zero-padding bits to reach the next byte boundary before VUI.
        while br.position() % 8 != 0 {
            br.read_u(1)?;
        }
        parse_h266_vui(br, vui_payload_size_minus1 + 1)?
    } else {
        (None, None)
    };

    // §7.3.2.4 — extension flag (consume; don't model range extensions).
    let _sps_extension_flag = br.read_bool()?;

    Ok((color_info, frame_rate))
}

/// Walk `dpb_parameters(MaxSubLayersMinus1, subLayerInfoFlag)` per §7.3.4.
/// The loop runs from `(subLayerInfoFlag ? 0 : MaxSubLayersMinus1)`
/// to `MaxSubLayersMinus1`, reading 3 ue(v) fields per layer.
fn walk_dpb_parameters(
    br: &mut BitReader<'_>,
    max_sublayers_minus1: u8,
    sublayer_info_flag: bool,
) -> Result<(), ParseError> {
    let start = if sublayer_info_flag {
        0u8
    } else {
        max_sublayers_minus1
    };
    for _ in start..=max_sublayers_minus1 {
        let _dpb_max_dec_pic_buffering_minus1 = br.read_ue()?;
        let _dpb_max_num_reorder_pics = br.read_ue()?;
        let _dpb_max_latency_increase_plus1 = br.read_ue()?;
    }
    Ok(())
}

/// Walk `general_timing_hrd_parameters()` (§7.3.5.1) followed by
/// `ols_timing_hrd_parameters(firstSubLayer, sps_max_sublayers_minus1)`
/// (§7.3.5.2). Called only when `sps_timing_hrd_params_present_flag = 1`.
///
/// Reads and discards `num_units_in_tick` / `time_scale` — Task 4.3 will
/// retrofit capture of those fields for `frame_rate` extraction.
fn walk_timing_hrd_parameters(
    br: &mut BitReader<'_>,
    max_sublayers_minus1: u8,
) -> Result<(), ParseError> {
    // §7.3.5.1 general_timing_hrd_parameters().
    let _num_units_in_tick = br.read_u(32)?;
    let _time_scale = br.read_u(32)?;
    let general_nal_hrd_params_present_flag = br.read_bool()?;
    let general_vcl_hrd_params_present_flag = br.read_bool()?;
    let (general_du_hrd_params_present_flag, hrd_cpb_cnt_minus1) =
        if general_nal_hrd_params_present_flag || general_vcl_hrd_params_present_flag {
            let _general_same_pic_timing_in_all_ols_flag = br.read_bool()?;
            let du_hrd = br.read_bool()?; // general_du_hrd_params_present_flag
            if du_hrd {
                let _tick_divisor_minus2 = br.read_u(8)?;
            }
            let _bit_rate_scale = br.read_u(4)?;
            let _cpb_size_scale = br.read_u(4)?;
            if du_hrd {
                let _cpb_size_du_scale = br.read_u(4)?;
            }
            let cpb_cnt = br.read_ue()?; // hrd_cpb_cnt_minus1
            (du_hrd, cpb_cnt)
        } else {
            (false, 0)
        };

    // §7.3.5.2 ols_timing_hrd_parameters(firstSubLayer, MaxSubLayersVal).
    let sublayer_cpb_params_present_flag = if max_sublayers_minus1 > 0 {
        br.read_bool()?
    } else {
        false
    };
    let first_sub_layer = if sublayer_cpb_params_present_flag {
        0u8
    } else {
        max_sublayers_minus1
    };
    for _ in first_sub_layer..=max_sublayers_minus1 {
        let fixed_pic_rate_general_flag = br.read_bool()?;
        let fixed_pic_rate_within_cvs_flag = if !fixed_pic_rate_general_flag {
            br.read_bool()?
        } else {
            true
        };
        if fixed_pic_rate_within_cvs_flag {
            let _elemental_duration_in_tc_minus1 = br.read_ue()?;
        } else if (general_nal_hrd_params_present_flag || general_vcl_hrd_params_present_flag)
            && hrd_cpb_cnt_minus1 == 0
        {
            let _low_delay_hrd_flag = br.read_bool()?;
        }
        // §7.3.5.3 sublayer_hrd_parameters(subLayerId).
        // Called once per sublayer-HRD direction that has params present.
        if general_nal_hrd_params_present_flag {
            walk_sublayer_hrd_parameters(
                br,
                hrd_cpb_cnt_minus1,
                general_du_hrd_params_present_flag,
            )?;
        }
        if general_vcl_hrd_params_present_flag {
            walk_sublayer_hrd_parameters(
                br,
                hrd_cpb_cnt_minus1,
                general_du_hrd_params_present_flag,
            )?;
        }
    }
    Ok(())
}

/// Walk `sublayer_hrd_parameters(subLayerId)` per §7.3.5.3.
/// Reads bit_rate_value_minus1, cpb_size_value_minus1 (and optional DU
/// variants) + cbr_flag for each CPB in `0..=hrd_cpb_cnt_minus1`.
fn walk_sublayer_hrd_parameters(
    br: &mut BitReader<'_>,
    hrd_cpb_cnt_minus1: u32,
    du_hrd_params_present: bool,
) -> Result<(), ParseError> {
    for _ in 0..=hrd_cpb_cnt_minus1 {
        let _bit_rate_value_minus1 = br.read_ue()?;
        let _cpb_size_value_minus1 = br.read_ue()?;
        if du_hrd_params_present {
            let _cpb_size_du_value_minus1 = br.read_ue()?;
            let _bit_rate_du_value_minus1 = br.read_ue()?;
        }
        let _cbr_flag = br.read_bool()?;
    }
    Ok(())
}

/// Walk `ref_pic_list_struct(listIdx, rplsIdx)` per §7.3.10.
///
/// Advances the cursor past one reference picture list structure.
/// Long-term ref-pic entries that signal `rpls_poc_lsb_lt` require a variable
/// bit width (`u(v)` with width derived from `sps_log2_max_pic_order_cnt_lsb_minus4`);
/// since that width is not passed into this helper, the long-term non-in-header
/// path bails to `UnsupportedProfile`. In practice `sps_long_term_ref_pics_flag`
/// is `false` on VVenC default output so this path is never taken on real fixtures.
fn walk_ref_pic_list_struct(
    br: &mut BitReader<'_>,
    _list_idx: u32,
    rpls_idx: u32,
    num_ref_pic_lists: u32,
    long_term_ref_pics_flag: bool,
    inter_layer_pred_enabled_flag: bool,
) -> Result<(), ParseError> {
    let num_ref_entries = br.read_ue()?;
    // ltrp_in_header_flag: present when long_term_ref_pics_flag is true,
    // rplsIdx < num_ref_pic_lists[listIdx], and num_ref_entries > 0.
    let ltrp_in_header_flag =
        if long_term_ref_pics_flag && rpls_idx < num_ref_pic_lists && num_ref_entries > 0 {
            br.read_bool()?
        } else {
            true // implied when not signalled
        };

    for _ in 0..num_ref_entries {
        let inter_layer_ref_pic_flag = if inter_layer_pred_enabled_flag {
            br.read_bool()?
        } else {
            false
        };
        if !inter_layer_ref_pic_flag {
            let st_ref_pic_flag = if long_term_ref_pics_flag {
                br.read_bool()? // st_ref_pic_flag[listIdx][rplsIdx][i]
            } else {
                true // short-term only when long_term_ref_pics=0
            };
            if st_ref_pic_flag {
                // Short-term entry: abs_delta_poc_st ue(v) +
                // optional sign bit when abs_delta_poc_st > 0.
                let abs_delta_poc_st = br.read_ue()?;
                if abs_delta_poc_st > 0 {
                    let _strp_entry_sign_flag = br.read_bool()?;
                }
            } else if ltrp_in_header_flag {
                // Long-term entry signalled in slice header — no bits here.
            } else {
                // Long-term entry signalled in RPLS — requires poc_lsb_lt of
                // width `log2_max_pic_order_cnt_lsb_minus4 + 4` bits. We don't
                // thread that width through the call stack, so bail here.
                return Err(ParseError::UnsupportedProfile { profile_idc: 0 });
            }
        } else {
            // Inter-layer ref-pic: ilrp_idx ue(v).
            let _ilrp_idx = br.read_ue()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::ChromaFormat;

    /// Inline bit-builder. Mirrors the parser's expected reads exactly,
    /// keeping the test bytes debuggable by reading the field-write
    /// sequence top-to-bottom.
    struct BitWriter {
        bytes: Vec<u8>,
        pos: u32,
    }

    impl BitWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                pos: 0,
            }
        }
        fn write(&mut self, value: u32, n: u32) {
            for i in (0..n).rev() {
                let bit = ((value >> i) & 1) as u8;
                let byte_idx = (self.pos / 8) as usize;
                let bit_in_byte = 7 - (self.pos % 8);
                if byte_idx == self.bytes.len() {
                    self.bytes.push(0);
                }
                self.bytes[byte_idx] |= bit << bit_in_byte;
                self.pos += 1;
            }
        }
        /// Exp-Golomb ue(v) per H.266 §9.3.2.2 (identical formula to H.264/H.265).
        fn write_ue(&mut self, value: u32) {
            let v = value + 1;
            let leading_zeros = 31 - v.leading_zeros();
            for _ in 0..leading_zeros {
                self.write(0, 1);
            }
            self.write(v, leading_zeros + 1);
        }
        /// rbsp_trailing_bits(): one '1' bit + zero-pad to byte align.
        fn end_rbsp(&mut self) {
            self.write(1, 1);
            while self.pos % 8 != 0 {
                self.write(0, 1);
            }
        }
    }

    /// Construct a minimal valid H.266 SPS bitstream:
    /// sps_id=0, vps_id=0, 320x240, 8-bit 4:2:0, Main 10 profile @ Level 4.0.
    ///
    /// Per H.266 V4 §7.3.2.4 SPS syntax + §7.3.3.1 PTL syntax.
    fn minimal_sps_rbsp() -> Vec<u8> {
        minimal_sps_rbsp_with_bitdepth_minus8(0)
    }

    /// Same as [`minimal_sps_rbsp`] but lets callers inject an arbitrary
    /// `sps_bitdepth_minus8` value to exercise the bounds check.
    fn minimal_sps_rbsp_with_bitdepth_minus8(bitdepth_minus8: u32) -> Vec<u8> {
        minimal_sps_rbsp_full(bitdepth_minus8, None)
    }

    /// Same minimal SPS but with `sps_conformance_window_flag = 1` and
    /// caller-supplied `(left, right, top, bottom)` offsets in
    /// SubWidthC/SubHeightC units. 4:2:0 chroma → SubWidthC=SubHeightC=2,
    /// so the surfaced `crop_*` luma-sample values are 2× the offsets.
    fn minimal_sps_rbsp_with_conformance_window(offsets: (u32, u32, u32, u32)) -> Vec<u8> {
        minimal_sps_rbsp_full(0, Some(offsets))
    }

    fn minimal_sps_rbsp_full(
        bitdepth_minus8: u32,
        conf_window: Option<(u32, u32, u32, u32)>,
    ) -> Vec<u8> {
        let mut bw = BitWriter::new();

        // §7.3.2.4 SPS header.
        bw.write(0, 4); // sps_seq_parameter_set_id
        bw.write(0, 4); // sps_video_parameter_set_id
        bw.write(0, 3); // sps_max_sublayers_minus1
        bw.write(1, 2); // sps_chroma_format_idc = 1 (4:2:0)
        bw.write(0, 2); // sps_log2_ctu_size_minus5
        bw.write(1, 1); // sps_ptl_dpb_hrd_params_present_flag = 1

        // §7.3.3.1 profile_tier_level(profileTierPresentFlag=1, MaxNumSubLayersMinus1=0).
        bw.write(1, 7); // general_profile_idc = 1 (Main 10)
        bw.write(0, 1); // general_tier_flag = 0 (Main tier)
        bw.write(63, 8); // general_level_idc = 63 (Level 4.0)
        bw.write(0, 1); // ptl_frame_only_constraint_flag
        bw.write(0, 1); // ptl_multilayer_enabled_flag
        // §7.3.3.2 general_constraints_info(): gci_present_flag=0 → only
        // the flag bit, then byte-align.
        bw.write(0, 1); // gci_present_flag = 0
        // Byte-align: bits written so far in PTL = 7+1+8+1+1+1 = 19,
        // need 5 zero bits to align to 24 bits = 3 bytes.
        bw.write(0, 5);
        // No sublayer ptl_sublayer_level_present_flag loop (count = -1).
        // No sublayer level_idc loop.
        bw.write(0, 8); // ptl_num_sub_profiles = 0
        // No sub_profile_idc loop.
        // PTL total = 32 bits = 4 bytes.

        // §7.3.2.4 continues.
        bw.write(0, 1); // sps_gdr_enabled_flag
        bw.write(0, 1); // sps_ref_pic_resampling_enabled_flag = 0
        // (sps_res_change_in_clvs_allowed_flag not coded.)

        // sps_pic_width_max_in_luma_samples is the direct sample count
        // (not minus-1 like H.264/H.265 use elsewhere). Per V4 §7.3.2.4.
        bw.write_ue(320); // sps_pic_width_max_in_luma_samples
        bw.write_ue(240); // sps_pic_height_max_in_luma_samples

        if let Some((left, right, top, bottom)) = conf_window {
            bw.write(1, 1); // sps_conformance_window_flag = 1
            bw.write_ue(left); // sps_conf_win_left_offset
            bw.write_ue(right); // sps_conf_win_right_offset
            bw.write_ue(top); // sps_conf_win_top_offset
            bw.write_ue(bottom); // sps_conf_win_bottom_offset
        } else {
            bw.write(0, 1); // sps_conformance_window_flag = 0
        }
        bw.write(0, 1); // sps_subpic_info_present_flag = 0
        bw.write_ue(bitdepth_minus8); // sps_bitdepth_minus8

        // ── body walk fields (Task 4.2) ──────────────────────────────────────
        // All flags set to 0 / false; all ue(v) values = 0.  This produces the
        // simplest valid SPS that the walk can traverse without any conditional
        // branches being taken.
        //
        // §7.3.2.4 continued (order mirrors walk_sps_body exactly):
        bw.write(0, 1); // sps_entropy_coding_sync_enabled_flag = 0
        bw.write(0, 1); // sps_entry_point_offsets_present_flag = 0
        bw.write(0, 4); // sps_log2_max_pic_order_cnt_lsb_minus4 = 0 (u4)
        bw.write(0, 1); // sps_poc_msb_cycle_flag = 0
        // (no sps_poc_msb_cycle_len_minus1)
        bw.write(0, 2); // sps_num_extra_ph_bytes = 0 (u2)
        // (no extra_ph_bit_present_flag loop)
        bw.write(0, 2); // sps_num_extra_sh_bytes = 0 (u2)
        // (no extra_sh_bit_present_flag loop)

        // ptl_dpb_hrd_params_present_flag = 1, sps_max_sublayers_minus1 = 0
        // → sps_sublayer_dpb_params_flag NOT read (only present when
        //   max_sublayers_minus1 > 0).
        // dpb_parameters(0, false): loop from 0 to 0 (one iteration).
        bw.write_ue(1); // dpb_max_dec_pic_buffering_minus1[0] = 1
        bw.write_ue(0); // dpb_max_num_reorder_pics[0] = 0
        bw.write_ue(0); // dpb_max_latency_increase_plus1[0] = 0

        bw.write_ue(0); // sps_log2_min_luma_coding_block_size_minus2 = 0
        // log2_ctu_size_minus5 = 0 → CTBsize = 32 (not > 32).

        bw.write(0, 1); // sps_partition_constraints_override_enabled_flag = 0

        // Intra luma partition constraints (always present).
        bw.write_ue(0); // sps_log2_diff_min_qt_min_cb_intra_slice_luma = 0
        bw.write_ue(0); // sps_max_mtt_hierarchy_depth_intra_slice_luma = 0
        // (depth=0 → no BT/TT max-diff fields)

        // chroma_format_idc = 1 (not 0) → sps_qtbtt_dual_tree_intra_flag present.
        bw.write(0, 1); // sps_qtbtt_dual_tree_intra_flag = 0
        // (dual_tree=0 → no chroma partition fields)

        // Inter partition constraints (always present).
        bw.write_ue(0); // sps_log2_diff_min_qt_min_cb_inter_slice = 0
        bw.write_ue(0); // sps_max_mtt_hierarchy_depth_inter_slice = 0
        // (depth=0 → no BT/TT max-diff fields)

        // CTBsize = 32 (not > 32) → sps_max_luma_transform_size_64_flag NOT present.

        bw.write(0, 1); // sps_transform_skip_enabled_flag = 0
        // (ts=0 → no log2_transform_skip + bdpcm fields)
        bw.write(0, 1); // sps_mts_enabled_flag = 0
        // (mts=0 → no intra/inter mts fields)
        bw.write(0, 1); // sps_lfnst_enabled_flag = 0

        // chroma_format_idc = 1 (not 0) → joint_cbcr + qp_table present.
        bw.write(0, 1); // sps_joint_cbcr_enabled_flag = 0
        bw.write(1, 1); // sps_same_qp_table_for_chroma_flag = 1
        // numQpTables = 1 (same_qp=1).
        // QP table [0]: qp_table_start_minus26=0, num_points=0, one (in,out) pair.
        // se(0) = ue(0) → codeword "1" (1 bit).
        bw.write_ue(0); // sps_qp_table_start_minus26[0] = se(0) → maps to ue(0)
        bw.write_ue(0); // sps_num_points_in_qp_table_minus1[0] = 0
        // for j in 0..=0: two ue(0) values.
        bw.write_ue(0); // sps_delta_qp_in_val_minus1[0][0] = 0
        bw.write_ue(0); // sps_delta_qp_diff_val[0][0] = 0

        bw.write(0, 1); // sps_sao_enabled_flag = 0
        bw.write(0, 1); // sps_alf_enabled_flag = 0
        // (alf=0 → no ccalf field)
        bw.write(0, 1); // sps_lmcs_enabled_flag = 0
        bw.write(0, 1); // sps_weighted_pred_flag = 0
        bw.write(0, 1); // sps_weighted_bipred_flag = 0
        bw.write(0, 1); // sps_long_term_ref_pics_flag = 0
        // vps_id = 0 → sps_inter_layer_prediction_enabled_flag NOT present.
        bw.write(0, 1); // sps_idr_rpl_present_flag = 0
        bw.write(0, 1); // sps_rpl1_same_as_rpl0_flag = 0
        // → loop 2 directions (L0 and L1).
        // L0: sps_num_ref_pic_lists[0] = 0 → no ref_pic_list_struct calls.
        bw.write_ue(0); // sps_num_ref_pic_lists[0] = 0
        // L1: sps_num_ref_pic_lists[1] = 0.
        bw.write_ue(0); // sps_num_ref_pic_lists[1] = 0

        bw.write(0, 1); // sps_ref_wraparound_enabled_flag = 0
        bw.write(0, 1); // sps_temporal_mvp_enabled_flag = 0
        // (tmvp=0 → no sbtmvp field)
        bw.write(0, 1); // sps_amvr_enabled_flag = 0
        bw.write(0, 1); // sps_bdof_enabled_flag = 0
        // (bdof=0 → no bdof_control_present_in_ph field)
        bw.write(0, 1); // sps_smvd_enabled_flag = 0
        bw.write(0, 1); // sps_dmvr_enabled_flag = 0
        // (dmvr=0 → no dmvr_control_present_in_ph field)
        bw.write(0, 1); // sps_mmvd_enabled_flag = 0
        // (mmvd=0 → no mmvd_fullpel_only field)
        // sps_six_minus_max_num_merge_cand = 0 → MaxNumMergeCand = 6.
        bw.write_ue(0); // sps_six_minus_max_num_merge_cand = 0
        bw.write(0, 1); // sps_sbt_enabled_flag = 0
        bw.write(0, 1); // sps_affine_enabled_flag = 0
        // (affine=0 → no affine sub-fields)
        bw.write(0, 1); // sps_bcw_enabled_flag = 0
        bw.write(0, 1); // sps_ciip_enabled_flag = 0
        // MaxNumMergeCand = 6 >= 2 → sps_gpm_enabled_flag present.
        bw.write(0, 1); // sps_gpm_enabled_flag = 0
        // (gpm=0 → no max_num_gpm_cand field)
        bw.write_ue(0); // sps_log2_parallel_merge_level_minus2 = 0
        bw.write(0, 1); // sps_isp_enabled_flag = 0
        bw.write(0, 1); // sps_mrl_enabled_flag = 0
        bw.write(0, 1); // sps_mip_enabled_flag = 0
        // chroma_format_idc = 1 (not 0) → sps_cclm_enabled_flag present.
        bw.write(0, 1); // sps_cclm_enabled_flag = 0
        // chroma_format_idc = 1 → sps_chroma_horiz/vert_collocated_flag present.
        bw.write(0, 1); // sps_chroma_horizontal_collocated_flag = 0
        bw.write(0, 1); // sps_chroma_vertical_collocated_flag = 0
        bw.write(0, 1); // sps_palette_enabled_flag = 0
        // chroma_format_idc=1 (not 3) → sps_act_enabled_flag NOT present.
        // ts=0, palette=0 → sps_min_qp_prime_ts NOT present.
        bw.write(0, 1); // sps_ibc_enabled_flag = 0
        // (ibc=0 → no six_minus_max_num_ibc field)
        bw.write(0, 1); // sps_ladf_enabled_flag = 0
        bw.write(0, 1); // sps_explicit_scaling_list_enabled_flag = 0
        // (explicit_scaling=0 → no scaling list fields)
        // lfnst=0, explicit_scaling=0 → no lfnst_disabled flag.
        // act=0, explicit_scaling=0 → no act_disabled flag.
        bw.write(0, 1); // sps_dep_quant_enabled_flag = 0
        bw.write(0, 1); // sps_sign_data_hiding_enabled_flag = 0
        bw.write(0, 1); // sps_virtual_boundaries_enabled_flag = 0

        // ptl_dpb_hrd_params_present_flag = 1 → sps_timing_hrd_params_present_flag present.
        bw.write(0, 1); // sps_timing_hrd_params_present_flag = 0
        // (timing_hrd=0 → no general_timing_hrd_parameters call)

        bw.write(0, 1); // sps_field_seq_flag = 0
        bw.write(0, 1); // sps_vui_parameters_present_flag = 0
        // (vui=0 → no vui_payload_size or vui_parameters call)

        bw.write(0, 1); // sps_extension_flag = 0

        bw.end_rbsp();
        bw.bytes
    }

    #[test]
    fn parse_sps_320x240_main10() {
        let rbsp = minimal_sps_rbsp();
        let sps = parse_sps(&rbsp).expect("minimal SPS should parse");
        assert_eq!(sps.sps_id, 0);
        assert_eq!(sps.vps_id, 0);
        assert_eq!(sps.profile_tier_level.general_profile_idc, 1);
        assert!(!sps.profile_tier_level.general_tier_flag);
        assert_eq!(sps.profile_tier_level.general_level_idc, 63);
        assert_eq!(sps.width, 320);
        assert_eq!(sps.height, 240);
        assert_eq!(sps.chroma_format, ChromaFormat::Yuv420);
        assert_eq!(sps.bit_depth_luma, 8);
        assert_eq!(sps.bit_depth_chroma, 8);
        assert_eq!(sps.color_info, None);
        assert_eq!(sps.frame_rate, None);
    }

    #[test]
    fn parse_sps_truncated_returns_err() {
        assert!(parse_sps(&[]).is_err());
    }

    #[test]
    fn parse_sps_truncated_byte_returns_err() {
        // Parser reads sps_id(4) + vps_id(4) = 8 bits, then max_sublayers(3)
        // beyond a single byte should bail with TruncatedRbsp.
        assert!(parse_sps(&[0x00]).is_err());
    }

    /// Per H.266 V4 §7.4.3.4, `sps_bitdepth_minus8 ∈ 0..=8` (bit_depth ∈
    /// 8..=16). ffmpeg's `libavcodec/hevc/ps.c:366-369` clamps at 14
    /// (minus8 ≤ 6); we adopt the same threshold. A fuzzed value of 248
    /// would have silently wrapped to `bit_depth_luma=0` via
    /// `8u8.saturating_add(248 as u8)` — caught now via
    /// [`validate_bit_depth_minus8`].
    /// Per H.266 V4 §7.4.3.4, conformance-window crop offsets are
    /// expressed in SubWidthC / SubHeightC units. For 4:2:0 chroma both
    /// are 2, so a (1, 2, 3, 4) offset tuple → luma crops (2, 4, 6, 8)
    /// and width/height shrink by (left+right) / (top+bottom).
    #[test]
    fn h266_sps_surfaces_conformance_window_offsets_invariant() {
        let rbsp = minimal_sps_rbsp_with_conformance_window((1, 2, 3, 4));
        let sps = parse_sps(&rbsp).expect("SPS with conformance window should parse");
        assert_eq!(sps.crop_left, 2);
        assert_eq!(sps.crop_right, 4);
        assert_eq!(sps.crop_top, 6);
        assert_eq!(sps.crop_bottom, 8);
        // pic_width_max=320, pic_height_max=240. After cropping:
        // width = 320 - (2+4) = 314, height = 240 - (6+8) = 226.
        assert_eq!(sps.width, 314);
        assert_eq!(sps.height, 226);
        assert_eq!(sps.coded_width(), 320);
        assert_eq!(sps.coded_height(), 240);
        assert_eq!(
            sps.coded_width(),
            sps.width + sps.crop_left + sps.crop_right
        );
        assert_eq!(
            sps.coded_height(),
            sps.height + sps.crop_top + sps.crop_bottom
        );
    }

    /// When `sps_conformance_window_flag=0`, all four crop offsets are
    /// zero and `coded_*` matches the cropped dimensions.
    #[test]
    fn h266_sps_no_conformance_window_zero_crops() {
        let sps = parse_sps(&minimal_sps_rbsp()).expect("minimal SPS should parse");
        assert_eq!(sps.crop_left, 0);
        assert_eq!(sps.crop_right, 0);
        assert_eq!(sps.crop_top, 0);
        assert_eq!(sps.crop_bottom, 0);
        assert_eq!(sps.coded_width(), sps.width);
        assert_eq!(sps.coded_height(), sps.height);
    }

    #[test]
    fn h266_sps_rejects_bit_depth_overflow() {
        let rbsp = minimal_sps_rbsp_with_bitdepth_minus8(248);
        let result = parse_sps(&rbsp);
        assert!(
            matches!(
                result,
                Err(ParseError::ReservedValue {
                    field: "sps_bitdepth_minus8",
                    value: 248
                })
            ),
            "expected ReservedValue, got {result:?}"
        );
    }

    #[test]
    fn real_vvenc_sps_currently_returns_no_frame_rate_no_color() {
        // Pre-Phase-4 baseline: the parser reads sps_id/vps_id, PTL,
        // dimensions, conformance window, and bit-depth, then stops
        // (the body between bit-depth and VUI is not yet walked). Real
        // VVenC output therefore parses cleanly but surfaces
        // frame_rate=None and color_info=None today. This test pins that
        // baseline; the Phase 4 body-walk + VUI task will flip the
        // frame_rate assertion to expect Some.
        let rbsp =
            include_bytes!("../../../tests/fixtures/codec/h266/h266_320x240_main10_real_sps.bin");
        let sps = parse_sps(rbsp).expect("real VVenC SPS should parse cleanly");
        assert_eq!(sps.width, 320);
        assert_eq!(sps.height, 240);
        // Baseline: frame_rate is None today. Phase 4 task 4.4 will
        // change this assertion to expect Some.
        assert!(
            sps.frame_rate.is_none(),
            "Phase 4 baseline; rewrite when body walk lands"
        );
    }
}
