//! Rough memory sizing for a preset, so the UI can warn before a launch
//! that the machine cannot hold.
//!
//! **This is an estimate, not a measurement.** It reads the parameter
//! count and quantisation out of the repo name, which is the only sizing
//! information a `models.ini` actually carries. It is deliberately
//! conservative in what it claims: a preset it cannot parse reports
//! `None` and is never flagged, because a wrong red warning is worse than
//! no warning.

/// Bits per weight, or `None` when the quantisation is not recognised.
///
/// The `_K_XL`-style variants land a little above their nominal bit-width,
/// hence 4.8 rather than 4.0.
///
/// **An unknown tag returns `None` rather than assuming Q4.** The old
/// fallback did assume it, which is how `prism-ml/Bonsai-27B-gguf:Q1_0`
/// came to be announced at 16.1 GiB against a real file of 3.5: `Q1_0`
/// matched nothing, fell through to 4.8 bits and was sized as though it
/// were a Q4. Guessing a bit-width is guessing the answer — a preset that
/// cannot be sized reports nothing, exactly as one with no parseable
/// parameter count does.
fn bits_per_weight(reference: &str) -> Option<f64> {
    // The quantisation is the tag after `:` where there is one. Searching
    // the whole reference would let a model *name* decide the answer: a
    // repo called `...-Q8-distill` has no bearing on how its `:Q4_K_M`
    // build is stored.
    let tag = super::hub::split_repo(reference).1.unwrap_or(reference);
    let tag = tag.to_ascii_uppercase();

    // `IQ<n>` variants fall out of this for free, since `IQ4_NL` contains
    // `Q4`. Checked in descending width so no entry shadows a wider one.
    const WIDTHS: [(&str, f64); 10] = [
        ("F32", 32.0),
        ("BF16", 16.0),
        ("F16", 16.0),
        ("Q8", 8.5),
        ("Q6", 6.6),
        ("Q5", 5.5),
        ("Q4", 4.8),
        ("Q3", 3.9),
        ("Q2", 2.8),
        // Measured, not nominal: Bonsai-27B-Q1_0.gguf is 3.54 GiB for
        // 27B weights, which is 1.13 bits each.
        ("Q1", 1.2),
    ];

    WIDTHS
        .iter()
        .find(|(needle, _)| tag.contains(needle))
        .map(|(_, bits)| *bits)
}

/// Runtime cost beyond the weights: KV cache at a typical context, plus
/// the server's own working set. A flat allowance rather than a modelled
/// one — the exact figure depends on layer count and cache dtype, which
/// the ini does not state.
const RUNTIME_OVERHEAD_GIB: f64 = 1.0;

const BYTES_PER_GIB: f64 = 1_073_741_824.0;

/// Parameter count in billions, read from a repo or preset name.
///
/// Takes the **largest** `<n>B` token so a MoE name like
/// `Qwen3.6-35B-A3B` reports 35 (all experts are resident) rather than the
/// 3B active count.
pub fn parameters_b(text: &str) -> Option<f64> {
    let bytes: Vec<char> = text.chars().collect();
    let mut best: Option<f64> = None;
    let mut index = 0;

    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }

        let start = index;
        while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == '.') {
            index += 1;
        }

        // Must be followed by a 'b'/'B' that is not part of a longer word.
        let is_b = matches!(bytes.get(index), Some('b') | Some('B'));
        let ends_token = !matches!(bytes.get(index + 1), Some(c) if c.is_ascii_alphanumeric());

        if is_b && ends_token {
            if let Ok(value) = bytes[start..index]
                .iter()
                .collect::<String>()
                .parse::<f64>()
            {
                if value > 0.0 && best.is_none_or(|current| value > current) {
                    best = Some(value);
                }
            }
        }
    }

    best
}

/// Estimated resident size of a preset, in GiB.
///
/// `None` when either half is unreadable — an unparseable parameter count
/// or an unrecognised quantisation. Both render as `-` and `Fit::Unknown`
/// rather than as a number nobody should act on.
pub fn estimate_gib(repo: &str) -> Option<f64> {
    let params = parameters_b(repo)?;
    let bits = bits_per_weight(repo)?;

    let weights = params * 1e9 * bits / 8.0 / BYTES_PER_GIB;
    Some(weights + RUNTIME_OVERHEAD_GIB)
}

/// How much of the machine's memory a model may use.
///
/// `reserved_ratio` is the share held back for the OS and everything else.
/// On Apple Silicon the system keeps a slice of unified memory for itself,
/// so the whole of `total_gib` is never addressable by the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    pub total_gib: f64,
    pub reserved_ratio: f64,
}

/// macOS keeps roughly a quarter of unified memory away from the GPU by
/// default, which is where this figure comes from.
pub const DEFAULT_RESERVED_RATIO: f64 = 0.25;
/// Below this the machine has very little left for anything else.
pub const MIN_RESERVED_RATIO: f64 = 0.05;
pub const MAX_RESERVED_RATIO: f64 = 0.60;

impl Budget {
    pub fn new(total_gib: f64, reserved_ratio: f64) -> Self {
        Self {
            total_gib,
            reserved_ratio: reserved_ratio.clamp(MIN_RESERVED_RATIO, MAX_RESERVED_RATIO),
        }
    }

    /// Memory a model may actually occupy.
    pub fn available_gib(&self) -> f64 {
        self.total_gib * (1.0 - self.reserved_ratio)
    }

    pub fn reserved_gib(&self) -> f64 {
        self.total_gib * self.reserved_ratio
    }

    /// True when the user has pushed the reservation below the default and
    /// so is running with less headroom than the system expects.
    pub fn is_risky(&self) -> bool {
        self.reserved_ratio < DEFAULT_RESERVED_RATIO
    }

    pub fn fit(&self, estimate_gib: Option<f64>) -> Fit {
        let Some(estimate) = estimate_gib else {
            return Fit::Unknown;
        };
        let available = self.available_gib();

        if estimate > available {
            Fit::TooLarge
        } else if estimate > available * 0.85 {
            Fit::Tight
        } else {
            Fit::Fits
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Comfortably within budget.
    Fits,
    /// Within budget but close enough that a long context may not be.
    Tight,
    /// Larger than the machine can give it.
    TooLarge,
    /// The preset carries no parseable size — say nothing rather than guess.
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_plain_parameter_count() {
        assert_eq!(parameters_b("unsloth/gemma-4-12B-it-qat-GGUF"), Some(12.0));
        assert_eq!(parameters_b("unsloth/Qwen3.5-4B-GGUF"), Some(4.0));
        assert_eq!(parameters_b("unsloth/Qwen3-14B-GGUF"), Some(14.0));
    }

    /// All experts of a MoE are resident, so the total count is what
    /// matters for memory — not the active count in the `A3B` suffix.
    #[test]
    fn a_mixture_of_experts_reports_its_total_not_its_active_count() {
        assert_eq!(
            parameters_b("unsloth/Qwen3.6-35B-A3B-MTP-GGUF:UD-Q4_K_XL"),
            Some(35.0)
        );
        assert_eq!(
            parameters_b("unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF"),
            Some(30.0)
        );
        assert_eq!(
            parameters_b("unsloth/gemma-4-26B-A4B-it-qat-GGUF"),
            Some(26.0)
        );
    }

    /// The bug this replaced: `Q1_0` matched no branch, fell through to a
    /// Q4-shaped default and was sized at 16.1 GiB against a real file of
    /// 3.54 GiB. The estimate adds a runtime allowance on top of the
    /// weights, so it lands near 4.5, not on the file size itself.
    #[test]
    fn a_one_bit_quantisation_is_not_sized_as_a_four_bit_one() {
        let estimate = estimate_gib("prism-ml/Bonsai-27B-gguf:Q1_0").expect("sizeable");

        // Bonsai-27B-Q1_0.gguf really is 3.54 GiB; allow the weights part
        // to land within half a gigabyte of that.
        let weights = estimate - RUNTIME_OVERHEAD_GIB;
        assert!(
            (weights - 3.54).abs() < 0.5,
            "weights estimated at {weights:.2} GiB against a real 3.54"
        );
        assert!(estimate < 6.0, "still sized like a Q4: {estimate:.1} GiB");
    }

    /// Guessing a bit-width is guessing the answer. An unrecognised
    /// quantisation reports nothing, exactly as an unparseable parameter
    /// count does.
    #[test]
    fn an_unknown_quantisation_is_not_assumed_to_be_q4() {
        assert_eq!(estimate_gib("vendor/Model-27B-GGUF:MXFP4"), None);
        assert_eq!(estimate_gib("vendor/Model-27B-GGUF:BITNET"), None);
        // No tag at all is equally unsizeable.
        assert_eq!(estimate_gib("vendor/Model-27B-GGUF"), None);
    }

    /// The quantisation comes from the tag, not the model name — a repo
    /// whose *name* mentions a width must not override the build's own.
    #[test]
    fn the_tag_decides_the_width_not_the_model_name() {
        let named = estimate_gib("vendor/Model-Q8-distill-27B-GGUF:Q1_0").expect("sizeable");
        let plain = estimate_gib("vendor/Model-27B-GGUF:Q1_0").expect("sizeable");

        assert!((named - plain).abs() < 0.01, "the name changed the answer");
    }

    /// `IQ4_NL`, `UD-IQ2_M`, `TQ2_0` and friends fall out of the same
    /// table: the `Q<n>` inside them is the width, and landing near it
    /// beats reporting nothing.
    #[test]
    fn the_i_quant_and_ternary_variants_are_recognised() {
        for tag in ["IQ4_NL", "UD-IQ2_M", "TQ2_0", "IQ1_S"] {
            assert!(
                estimate_gib(&format!("unsloth/Qwen3.5-9B-GGUF:{tag}")).is_some(),
                "{tag} was not recognised"
            );
        }
    }

    /// Quantisation tags contain digits too; none of them may be mistaken
    /// for a parameter count.
    #[test]
    fn quantisation_tags_are_not_parameter_counts() {
        assert_eq!(parameters_b("something-GGUF:UD-Q4_K_XL"), None);
        assert_eq!(parameters_b("something-GGUF:Q4_K_M"), None);
        assert_eq!(parameters_b("unsloth/Phi-4-mini-instruct-GGUF"), None);
    }

    #[test]
    fn an_unsized_preset_estimates_nothing() {
        assert_eq!(estimate_gib("unsloth/Phi-4-mini-instruct-GGUF"), None);
    }

    /// Sanity-checked against the VRAM column of the user's own
    /// `16gb/LLM hosting.md`: Qwen 3.5 4B ~2.4 GB, 9B ~5.5 GB.
    #[test]
    fn estimates_land_near_the_published_figures() {
        let four = estimate_gib("unsloth/Qwen3.5-4B-GGUF:UD-Q4_K_XL").expect("4B");
        let nine = estimate_gib("unsloth/Qwen3.5-9B-GGUF:UD-Q4_K_XL").expect("9B");

        assert!((3.0..4.0).contains(&four), "4B estimated at {four}");
        assert!((5.5..7.0).contains(&nine), "9B estimated at {nine}");
    }

    #[test]
    fn heavier_quantisation_estimates_larger() {
        let q4 = estimate_gib("model-12B-GGUF:Q4_K_M").expect("q4");
        let q8 = estimate_gib("model-12B-GGUF:Q8_0").expect("q8");
        assert!(q8 > q4 * 1.5, "q4={q4} q8={q8}");
    }

    #[test]
    fn a_budget_holds_back_the_reserved_share() {
        let budget = Budget::new(36.0, 0.25);
        assert_eq!(budget.available_gib(), 27.0);
        assert_eq!(budget.reserved_gib(), 9.0);
        assert!(!budget.is_risky());
    }

    #[test]
    fn the_reserved_ratio_is_clamped_to_a_sane_range() {
        assert_eq!(Budget::new(36.0, 0.0).reserved_ratio, MIN_RESERVED_RATIO);
        assert_eq!(Budget::new(36.0, 0.99).reserved_ratio, MAX_RESERVED_RATIO);
    }

    #[test]
    fn reserving_less_than_the_default_is_flagged_as_risky() {
        assert!(Budget::new(36.0, 0.10).is_risky());
        assert!(!Budget::new(36.0, DEFAULT_RESERVED_RATIO).is_risky());
    }

    /// The case the warning exists for: a 31B preset on a 16 GiB machine.
    #[test]
    fn a_model_larger_than_the_machine_is_too_large() {
        let small = Budget::new(16.0, DEFAULT_RESERVED_RATIO);
        let big = estimate_gib("unsloth/gemma-4-31B-it-qat-GGUF:UD-Q4_K_XL");

        assert_eq!(small.fit(big), Fit::TooLarge);
    }

    #[test]
    fn the_same_model_fits_a_larger_machine() {
        let large = Budget::new(36.0, DEFAULT_RESERVED_RATIO);
        let big = estimate_gib("unsloth/gemma-4-31B-it-qat-GGUF:UD-Q4_K_XL");

        assert_eq!(large.fit(big), Fit::Fits);
    }

    #[test]
    fn a_preset_of_unknown_size_is_never_flagged() {
        let budget = Budget::new(8.0, DEFAULT_RESERVED_RATIO);
        assert_eq!(budget.fit(None), Fit::Unknown);
    }

    /// Freeing up reserved memory can bring a model into range — which is
    /// exactly why the override exists, and why it carries a caution.
    #[test]
    fn lowering_the_reservation_can_make_a_model_fit() {
        let model = estimate_gib("unsloth/Qwen3.6-35B-A3B-MTP-GGUF:UD-Q4_K_XL");

        assert_eq!(Budget::new(24.0, 0.25).fit(model), Fit::TooLarge);
        assert_eq!(Budget::new(24.0, 0.05).fit(model), Fit::Tight);
    }
}
