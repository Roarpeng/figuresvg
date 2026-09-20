//! spec.json I/O (spec_version 2) + PARAMS defaults with overrides.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const PARAMS_DEFAULTS: &[(&str, f64)] = &[
    ("noise_whiten_min", 240.0),
    ("noise_whiten_spread", 12.0),
    ("noise_guard_radius", 8.0),
    ("region_min_area_frac", 4e-5),
    ("marker_max_area_frac", 3e-4),
    ("flat_spread", 22.0),
    ("gradient_axiality", 0.90),
    ("gradient_strips", 48.0),
    ("gap_merge_radius", 3.0),
    ("color_split_bins", 24.0),
    ("circularity_min", 0.72),
    ("rect_fill_ratio", 0.985),
    ("black_lum_max", 150.0),
    ("black_spread_max", 25.0),
    ("black_min_component_area", 45.0),
    // palette-layers strategy (TypeSafe-decided): uniform quantization step
    // in RGB units; ramp steps of gradients become distinct palette entries
    ("noise_max_blob_frac", 2e-4),
    ("engine", 0.0), // 0 = palette layers (default), 1 = legacy estimator
    ("objects", 1.0), // scene mode: 1 = object consolidation (v2), 0 = per-CC legacy
    ("palette_step", 8.0),
    ("palette_min_count", 8.0),
    ("layer_min_subarea", 4.0),
];

// verify thresholds (fixed, not spec-overridable -- python parity):
// glyph_survival_min 0.95, global_badfrac_max 0.015,
// region_color_max_dev 8.0, halo_max_frac 2e-4 (see verify.rs)

#[derive(Clone, Serialize, Deserialize)]
pub struct RowSpec {
    pub top: i64,
    pub bottom: i64,
    pub x0: i64,
    pub x1: i64,
    #[serde(default = "default_ref_slice")]
    pub ref_slice: [i64; 2],
}

fn default_ref_slice() -> [i64; 2] {
    [10, 45]
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Recolor {
    pub row: usize,
    pub donor_rows: Vec<usize>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Spec {
    pub spec_version: i32,
    pub canvas: [i64; 2],
    #[serde(default)]
    pub layout: String,
    #[serde(default)]
    pub params: HashMap<String, f64>,
    #[serde(default)]
    pub recolor: Vec<Recolor>,
    #[serde(default)]
    pub rows: Vec<RowSpec>,
    #[serde(default)]
    pub out_of_scope: bool,
    #[serde(default)]
    pub force: bool,
}

impl Spec {
    pub fn p(&self, key: &str) -> f64 {
        if let Some(v) = self.params.get(key) {
            return *v;
        }
        PARAMS_DEFAULTS
            .iter()
            .find(|(k, _)| *k == key)
            .map(|&(_, v)| v)
            .unwrap_or(0.0)
    }

    pub fn from_json(s: &str) -> Result<Spec, String> {
        serde_json::from_str(s).map_err(|e| format!("bad spec: {e}"))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}
