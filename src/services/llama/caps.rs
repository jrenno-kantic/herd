//! What a preset was optimised for, and what it can do.
//!
//! Both are read from the two things a `models.ini` actually carries: the
//! repo reference, and the preset's own keys. Nothing is inferred from
//! vendor knowledge that would rot — "Qwen3 supports thinking" is true
//! today and a lie the moment a Qwen4 lands.
//!
//! `reasoning = off` is deliberately *not* treated as a capability. It is
//! set on every preset in both shipped tiers, so a column driven by it
//! would read the same on every row — a column that never varies is
//! decoration. It is a setting, and the Settings screen is where settings
//! live.

/// An optimisation baked into the weights themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Optimisation {
    /// Quantisation-aware training: quantised during training rather than
    /// after it, so it loses less than the bit-width suggests.
    Qat,
    /// Unsloth Dynamic — per-layer quantisation, the `UD-` tag prefix.
    Dynamic,
    /// Mixture of experts: the `A3B`/`A4B` in `35B-A3B` is the count of
    /// *active* parameters. Worth surfacing because the memory estimate
    /// sizes on the total, not the active count — all the experts are
    /// resident.
    MixtureOfExperts,
}

impl Optimisation {
    /// Three or four characters, because this shares a row with five other
    /// columns.
    pub fn short(self) -> &'static str {
        match self {
            Optimisation::Qat => "qat",
            Optimisation::Dynamic => "ud",
            Optimisation::MixtureOfExperts => "moe",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Optimisation::Qat => "quantisation-aware training",
            Optimisation::Dynamic => "Unsloth dynamic quantisation",
            Optimisation::MixtureOfExperts => "mixture of experts",
        }
    }
}

/// Something a preset can do, beyond generating text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Vision,
    /// Speculative decoding via a draft or MTP head.
    Speculative,
    Audio,
    Code,
}

impl Capability {
    pub fn letter(self) -> char {
        match self {
            Capability::Vision => 'V',
            Capability::Speculative => 'S',
            Capability::Audio => 'A',
            Capability::Code => 'C',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Capability::Vision => "vision",
            Capability::Speculative => "speculative decoding",
            Capability::Audio => "audio",
            Capability::Code => "code",
        }
    }
}

/// A capability the model has, and whether this preset switches it on.
///
/// The distinction matters: `no-mmproj = true` on a multimodal model is
/// not "no vision", it is "vision, deliberately off to save the projector's
/// memory". Showing those two the same way would hide a setting the user
/// chose and might want back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trait {
    pub capability: Capability,
    pub enabled: bool,
}

impl Trait {
    /// Uppercase when on, lowercase when the model has it but the preset
    /// does not use it.
    pub fn letter(self) -> char {
        if self.enabled {
            self.capability.letter()
        } else {
            self.capability.letter().to_ascii_lowercase()
        }
    }

    /// `vision` / `vision (off)`, for spelling out the selected row.
    pub fn label(self) -> String {
        if self.enabled {
            self.capability.label().to_string()
        } else {
            format!("{} (off)", self.capability.label())
        }
    }
}

/// Optimisations named by a repo reference.
pub fn optimisations(repo: &str) -> Vec<Optimisation> {
    let lower = repo.to_ascii_lowercase();
    let mut found = Vec::new();

    if lower.contains("-qat") || lower.contains("qat-") {
        found.push(Optimisation::Qat);
    }
    // The `UD-` tag prefix, e.g. `...GGUF:UD-Q4_K_XL`.
    if super::hub::split_repo(repo)
        .1
        .is_some_and(|tag| tag.to_ascii_uppercase().starts_with("UD-"))
    {
        found.push(Optimisation::Dynamic);
    }
    if is_moe(&lower) {
        found.push(Optimisation::MixtureOfExperts);
    }

    found
}

/// `35B-A3B`, `26B-A4B` — an active-parameter count marks a MoE.
///
/// Requires the digits to sit between `a` and `b` so that ordinary words
/// cannot trip it: `A4B` counts, `Nano-4B` does not.
fn is_moe(lower: &str) -> bool {
    let bytes: Vec<char> = lower.chars().collect();

    bytes
        .windows(3)
        .any(|window| window[0] == 'a' && window[1].is_ascii_digit() && window[2] == 'b')
        || bytes.windows(4).any(|window| {
            window[0] == 'a'
                && window[1].is_ascii_digit()
                && window[2].is_ascii_digit()
                && window[3] == 'b'
        })
}

/// What a preset can do, given its repo and its own keys.
///
/// `no_mmproj` and `spec_type` come from the ini rather than being guessed,
/// because they are what decides whether a capability is actually in use.
pub fn capabilities(repo: &str, no_mmproj: bool, spec_type: Option<&str>) -> Vec<Trait> {
    let lower = repo.to_ascii_lowercase();
    let mut found = Vec::new();

    // Only a model that *names* itself multimodal is called multimodal.
    //
    // `no-mmproj = true` is deliberately not treated as evidence of a
    // projector, though it is tempting: you would only disable one you
    // had. In practice it is set defensively across the shipped tiers,
    // including on `Phi-4-mini` and `Nemotron-3-Nano`, which have no
    // vision at all — reading it as a capability put a `v` against four
    // text-only models. It only decides whether a capability found by
    // other means is switched on.
    //
    // The cost is under-claiming: `gemma-4` ships an `mmproj` but does not
    // say so in its name, so it reads as text-only here. That is the right
    // direction to be wrong in, and the honest fix would be to look for an
    // `mmproj` in the repo listing rather than to guess harder.
    if lower.contains("-vl-") || lower.contains("vision") || lower.contains("omni") {
        found.push(Trait {
            capability: Capability::Vision,
            enabled: !no_mmproj,
        });
    }

    // The repo shipping an MTP head means the model can do it; the preset
    // setting `spec-type` means it does.
    let has_head = lower.contains("mtp") || lower.contains("eagle") || lower.contains("draft");
    let uses_head = spec_type.is_some_and(|spec| !spec.eq_ignore_ascii_case("none"));
    if has_head || uses_head {
        found.push(Trait {
            capability: Capability::Speculative,
            enabled: uses_head,
        });
    }

    if lower.contains("audio") || lower.contains("voice") || lower.contains("omni") {
        found.push(Trait {
            capability: Capability::Audio,
            enabled: true,
        });
    }

    if lower.contains("coder") || lower.contains("-code") {
        found.push(Trait {
            capability: Capability::Code,
            enabled: true,
        });
    }

    found
}

/// The compact column: `Vs`, `vS`, `-`.
pub fn letters(traits: &[Trait]) -> String {
    if traits.is_empty() {
        return "-".to_string();
    }
    traits.iter().map(|t| t.letter()).collect()
}

/// The compact column for optimisations: `qat ud`, `-`.
pub fn tokens(optimisations: &[Optimisation]) -> String {
    if optimisations.is_empty() {
        return "-".to_string();
    }
    optimisations
        .iter()
        .map(|o| o.short())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every case below is a real reference from the shipped tiers.
    #[test]
    fn qat_is_read_from_the_repo_name() {
        assert_eq!(
            optimisations("unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL"),
            vec![Optimisation::Qat, Optimisation::Dynamic]
        );
        assert!(!optimisations("unsloth/Qwen3-14B-GGUF:UD-Q4_K_XL").contains(&Optimisation::Qat));
    }

    #[test]
    fn the_ud_prefix_is_read_from_the_quant_tag_not_the_repo() {
        assert!(optimisations("unsloth/Qwen3-14B-GGUF:UD-Q4_K_XL").contains(&Optimisation::Dynamic));
        assert!(!optimisations("Qwen/Qwen3-4B-GGUF:Q4_K_M").contains(&Optimisation::Dynamic));
        assert!(!optimisations("prism-ml/Bonsai-27B-gguf:Q1_0").contains(&Optimisation::Dynamic));
    }

    /// The active-parameter count is what marks a mixture of experts, and
    /// it must not fire on an ordinary size like `Nano-4B`.
    #[test]
    fn mixture_of_experts_is_read_from_the_active_parameter_count() {
        for moe in [
            "unsloth/Qwen3.6-35B-A3B-MTP-GGUF:UD-Q4_K_XL",
            "unsloth/gemma-4-26B-A4B-it-qat-GGUF:UD-Q4_K_XL",
            "unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:UD-Q4_K_XL",
        ] {
            assert!(
                optimisations(moe).contains(&Optimisation::MixtureOfExperts),
                "{moe} was not recognised as MoE"
            );
        }

        for dense in [
            "nvidia/NVIDIA-Nemotron-3-Nano-4B-GGUF:Q4_K_M",
            "unsloth/Qwen3-14B-GGUF:UD-Q4_K_XL",
            "prism-ml/Bonsai-27B-gguf:Q1_0",
        ] {
            assert!(
                !optimisations(dense).contains(&Optimisation::MixtureOfExperts),
                "{dense} was wrongly called MoE"
            );
        }
    }

    /// Regression: `no-mmproj = true` is set defensively across the
    /// shipped tiers, including on text-only models. Reading it as
    /// evidence of a projector put a `v` against `Phi-4-mini` and
    /// `Nemotron-3-Nano`, which have no vision at all.
    #[test]
    fn no_mmproj_alone_never_claims_vision() {
        for text_only in [
            "unsloth/Phi-4-mini-instruct-GGUF:Q4_K_M",
            "nvidia/NVIDIA-Nemotron-3-Nano-4B-GGUF:Q4_K_M",
            "Qwen/Qwen3-4B-GGUF:Q4_K_M",
            "prism-ml/Bonsai-27B-gguf:Q1_0",
        ] {
            assert!(
                capabilities(text_only, true, None)
                    .iter()
                    .all(|t| t.capability != Capability::Vision),
                "{text_only} wrongly claims vision"
            );
        }
    }

    /// On a model that *is* multimodal, the flag still decides whether the
    /// projector is in use — off is a setting to get back, not an absence.
    #[test]
    fn a_disabled_projector_reads_as_vision_off() {
        let traits = capabilities("unsloth/Qwen3-VL-8B-Instruct-GGUF", true, None);
        let vision = traits
            .iter()
            .find(|t| t.capability == Capability::Vision)
            .expect("a VL model has vision");

        assert!(!vision.enabled);
        assert_eq!(vision.letter(), 'v');
        assert_eq!(vision.label(), "vision (off)");
    }

    #[test]
    fn a_vision_model_that_uses_its_projector_reports_it_enabled() {
        let traits = capabilities("unsloth/Qwen3-VL-8B-Instruct-GGUF", false, None);
        let vision = traits
            .iter()
            .find(|t| t.capability == Capability::Vision)
            .expect("vision");

        assert!(vision.enabled);
        assert_eq!(vision.letter(), 'V');
    }

    /// Shipping an MTP head and using one are different facts.
    #[test]
    fn speculative_distinguishes_having_a_head_from_using_it() {
        let idle = capabilities("unsloth/Qwen3.5-9B-MTP-GGUF", false, None);
        let spec = idle
            .iter()
            .find(|t| t.capability == Capability::Speculative)
            .expect("head present");
        assert!(!spec.enabled, "a head that is not configured is not in use");

        let active = capabilities("unsloth/Qwen3.5-9B-MTP-GGUF", false, Some("draft-mtp"));
        assert!(
            active
                .iter()
                .find(|t| t.capability == Capability::Speculative)
                .expect("head")
                .enabled
        );
    }

    /// `spec-type = none` is llama.cpp's way of saying "off", and must not
    /// read as speculative decoding being on.
    #[test]
    fn spec_type_none_is_not_speculative_decoding() {
        let traits = capabilities("unsloth/Qwen3-14B-GGUF", false, Some("none"));
        assert!(traits
            .iter()
            .all(|t| t.capability != Capability::Speculative));
    }

    #[test]
    fn a_coder_model_is_marked_as_one() {
        let traits = capabilities("unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF", false, None);
        assert!(traits.iter().any(|t| t.capability == Capability::Code));
    }

    /// Nothing in the shipped tiers has audio. The detector exists so a
    /// model that does lights up without another code change, but it must
    /// not fire on anything currently present.
    #[test]
    fn no_shipped_preset_claims_audio() {
        for repo in [
            "unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL",
            "unsloth/Qwen3-VL-8B-Instruct-GGUF:UD-Q4_K_XL",
            "unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:UD-Q4_K_XL",
            "nvidia/NVIDIA-Nemotron-3-Nano-4B-GGUF:Q4_K_M",
        ] {
            assert!(
                capabilities(repo, false, None)
                    .iter()
                    .all(|t| t.capability != Capability::Audio),
                "{repo} wrongly claims audio"
            );
        }

        assert!(capabilities("some/Qwen3-Omni-GGUF", false, None)
            .iter()
            .any(|t| t.capability == Capability::Audio));
    }

    /// A preset with nothing to report shows a dash rather than a blank,
    /// so an empty column is visibly empty rather than looking clipped.
    #[test]
    fn nothing_to_report_reads_as_a_dash() {
        assert_eq!(tokens(&[]), "-");
        assert_eq!(letters(&[]), "-");
    }

    #[test]
    fn the_compact_columns_stay_narrow() {
        let opts = optimisations("unsloth/gemma-4-26B-A4B-it-qat-GGUF:UD-Q4_K_XL");
        assert_eq!(tokens(&opts), "qat ud moe");

        let traits = capabilities("unsloth/Qwen3-VL-Coder-MTP-GGUF", true, Some("draft-mtp"));
        assert_eq!(letters(&traits), "vSC");
    }
}
