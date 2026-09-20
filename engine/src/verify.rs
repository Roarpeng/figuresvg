//! Four automatic acceptance checks (same thresholds as python PARAMS["verify"]).

use crate::cc::connected_components;
use crate::img::Img;
use crate::mask::{dilate, erode, Mask};

// PARAMS["verify"] thresholds (fixed, not spec-overridable -- python parity)
const V_GLYPH_MIN: f64 = 0.95;
const V_GLOBAL_MAX: f64 = 0.015;
const V_HALO_MAX: f64 = 2e-4;
const V_REGION_MAX: f64 = 8.0;

pub fn verify(
    render_png: &str,
    rep: &Img,
    content: &Mask,
    black_cc_src: usize,
    glyph_mask: Option<&Mask>,
) -> Result<Vec<(String, bool, String)>, String> {
    let ren = Img::load(render_png)?;
    if ren.w != rep.w || ren.h != rep.h {
        return Err(format!(
            "render size {}x{} != repainted {}x{}",
            ren.w, ren.h, rep.w, rep.h
        ));
    }
    let mut out: Vec<(String, bool, String)> = Vec::new();

    // 1. glyph survival --------------------------------------------------
    let mut ren_dark = Mask::new(ren.w, ren.h);
    let mut near: Option<Mask> = None;
    if let Some(gm) = glyph_mask {
        near = Some(dilate(gm, 4)); // 9x9
    }
    for y in 0..ren.h {
        for x in 0..ren.w {
            let c = ren.at(x, y);
            let dark = (c[0] + c[1] + c[2]) / 3.0 < 100.0;
            let n = near.as_ref().map(|m| m.bits[y * ren.w + x] != 0).unwrap_or(true);
            ren_dark.bits[y * ren.w + x] = (dark && n) as u8;
        }
    }
    let ncc = connected_components(&ren_dark).count;
    let ratio = ncc.min(black_cc_src) as f64 / (black_cc_src as f64).max(1.0);
    out.push((
        "glyph_survival".into(),
        ratio >= V_GLYPH_MIN,
        format!("render/src CC = {}/{} = {ratio:.3}", ncc, black_cc_src),
    ));

    // 2. global fidelity --------------------------------------------------
    let n_px = ren.w * ren.h;
    let interior = {
        let d = dilate(content, 2); // 5x5
        let mut m = Mask::new(ren.w, ren.h);
        for i in 0..n_px {
            m.bits[i] = (d.bits[i] == 0) as u8;
        }
        m
    };
    let mut dmax = vec![0.0f32; n_px];
    let mut bad = 0usize;
    let mut interior_count = 0usize;
    let mut gmd = 0.0f64;
    for y in 0..ren.h {
        for x in 0..ren.w {
            let (rc, pc) = (ren.at(x, y), rep.at(x, y));
            let d = (rc[0] - pc[0])
                .abs()
                .max((rc[1] - pc[1]).abs())
                .max((rc[2] - pc[2]).abs());
            dmax[y * ren.w + x] = d;
            gmd += d as f64;
            if interior.bits[y * ren.w + x] != 0 {
                interior_count += 1;
                if d > 60.0 {
                    bad += 1;
                }
            }
        }
    }
    let badfrac = if interior_count > 0 { bad as f64 / interior_count as f64 } else { 0.0 };
    let gmd = gmd / n_px as f64;
    out.push((
        "global_fidelity".into(),
        badfrac <= V_GLOBAL_MAX,
        format!("dev>60 on {:.2}% of interior px (mean {gmd:.2})", badfrac * 100.0),
    ));

    // 3. halo -------------------------------------------------------------
    let far = {
        let d = dilate(content, 4); // 9x9
        let mut m = Mask::new(ren.w, ren.h);
        for i in 0..n_px {
            m.bits[i] = (d.bits[i] == 0) as u8;
        }
        m
    };
    let mut halo = 0usize;
    let mut far_count = 0usize;
    for y in 0..ren.h {
        for x in 0..ren.w {
            if far.bits[y * ren.w + x] == 0 {
                continue;
            }
            far_count += 1;
            let (rc, pc) = (ren.at(x, y), rep.at(x, y));
            let src_white = pc[0].min(pc[1]).min(pc[2]) > 250.0;
            let rmx = rc[0].max(rc[1]).max(rc[2]);
            let rmn = rc[0].min(rc[1]).min(rc[2]);
            let ren_tint = (rmx - rmn > 40.0) && (rmn > 100.0);
            if src_white && ren_tint {
                halo += 1;
            }
        }
    }
    let frac = halo as f64 / (far_count as f64).max(1.0);
    out.push((
        "halo_free".into(),
        frac <= V_HALO_MAX,
        format!("{halo} px ({frac:.2e} of far-field)"),
    ));

    // 4. region color fidelity ---------------------------------------------
    let mut core = erode(content, 3); // 7x7
    if core.count() < content.count() * 3 / 10 {
        core = content.clone();
    }
    let mut devs: Vec<f32> = Vec::with_capacity(core.count());
    for y in 0..ren.h {
        for x in 0..ren.w {
            if core.bits[y * ren.w + x] != 0 {
                let (rc, pc) = (ren.at(x, y), rep.at(x, y));
                devs.push(
                    (rc[0] - pc[0])
                        .abs()
                        .max((rc[1] - pc[1]).abs())
                        .max((rc[2] - pc[2]).abs()),
                );
            }
        }
    }
    let dev = crate::stats::median(&devs);
    out.push((
        "region_color".into(),
        dev <= V_REGION_MAX,
        format!("median dev on content core = {dev:.2}/255"),
    ));
    Ok(out)
}
