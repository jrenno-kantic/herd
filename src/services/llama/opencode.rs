//! The `opencode.json` provider block for a preset.
//!
//! OpenCode reaches a local model through a custom provider backed by
//! `@ai-sdk/openai-compatible`, pointed at an OpenAI-compatible `/v1`
//! endpoint — which is exactly what `llama-server` serves. Everything
//! that block needs is already on the Models screen: the endpoint comes
//! from `[server]`, the model id is the `--alias` the launch passes, and
//! the rest of the fields are facts the ini states. Writing it out by
//! hand means reading a port off one pane and an alias off another, which
//! is where the typo that makes OpenCode talk to nothing comes from.
//!
//! **Built from the launch argv, not from the ini directly.** The argv is
//! where `[server] → [*] → [model] → mono-focus → overrides → CLI` has
//! already been resolved, so a `--ctx-size` changed on the Settings screen
//! reaches this block the same way it reaches the process — the same
//! reasoning that put the clipboard's shell command behind
//! `LaunchSettings::argv` rather than behind the rendered preview.
//!
//! ## What is claimed, and what is left out
//!
//! Every field here is something the preset actually says. A field herd
//! cannot support is **omitted rather than guessed**, so OpenCode falls
//! back to its own default instead of acting on a confident wrong answer —
//! the same restraint as `Fit::Unknown` and `Sizing::Estimated`:
//!
//! - `tool_call` is set only when `--jinja` is in the argv. Jinja
//!   templating is what makes `llama-server` parse a model's tool-call
//!   syntax into OpenAI tool calls; without it the endpoint answers, and
//!   OpenCode — which is a coding agent — cannot use a single tool.
//! - `reasoning` follows `--reasoning`/`--reasoning-format`: `off` and
//!   `none` are `false`, any other value is `true`, an absent flag says
//!   nothing.
//! - `attachment` is set only when the preset has vision *switched on*,
//!   read from `caps.rs`. That detection is documented as under-claiming
//!   rather than over-claiming, which is the right direction here too: a
//!   `true` on a text-only model is an upload that fails at the server.
//! - `limit` needs both halves or neither, because OpenCode's schema
//!   requires both. `context` is `--ctx-size`. `output` is `--n-predict`
//!   when the preset caps it, and the context size when it does not —
//!   llama-server's own default is "generate until the context is full",
//!   so that is the ceiling it will really enforce, not an invented one.

use super::caps::{Capability, Trait};

/// Where OpenCode reads its global config. Shown above the block so the
/// answer to "and where does this go?" is not a second search.
pub const CONFIG_PATH: &str = "~/.config/opencode/opencode.json";

/// The schema OpenCode publishes, which its editor tooling keys off.
pub const SCHEMA: &str = "https://opencode.ai/config.json";

/// The npm package that adapts an OpenAI-compatible endpoint. Fixed by
/// OpenCode, not a choice of ours.
pub const NPM: &str = "@ai-sdk/openai-compatible";

/// The provider key, and how it reads in OpenCode's model picker.
///
/// One provider for every preset rather than one per model: they all
/// answer on the same endpoint, and `models` is a map precisely so a
/// second preset is one more entry rather than a second block. Pasting
/// two of these therefore merges cleanly by hand.
pub const PROVIDER_ID: &str = "herd";
pub const PROVIDER_NAME: &str = "herd (llama-server)";

/// The `provider` block for one preset, as facts rather than as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    /// The OpenAI-compatible base, `/v1` included.
    pub base_url: String,
    /// What the model answers to at `/v1/models` — its `--alias`.
    pub model_id: String,
    pub tool_call: Option<bool>,
    pub reasoning: Option<bool>,
    pub attachment: Option<bool>,
    pub limit: Option<Limit>,
}

/// OpenCode's `limit` object. Both fields or neither: its schema makes
/// `context` and `output` required together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limit {
    pub context: u64,
    pub output: u64,
}

impl Provider {
    /// Reads a block out of the endpoint, the preset name, the argv a
    /// launch would spawn, and what `caps.rs` makes of the preset.
    pub fn from_launch(base_url: &str, preset: &str, argv: &[String], traits: &[Trait]) -> Self {
        // `--alias` is what llama-server reports at `/v1/models`, so it is
        // what OpenCode has to ask for. The preset name is the fallback
        // because the shipped tiers set the two to the same string — but
        // an ini that did not would break silently, and the alias is the
        // half that is true.
        let model_id = value(argv, &["--alias"])
            .unwrap_or(preset)
            .trim()
            .to_string();

        let context = value(argv, &["--ctx-size", "-c"]).and_then(|v| v.trim().parse::<u64>().ok());
        // A `--n-predict` of -1 is llama-server's "no cap", which parses
        // as no number here and falls through to the context size. That is
        // the same answer, arrived at without a special case.
        let predict = value(argv, &["--n-predict", "--predict", "-n"])
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|n| *n > 0);

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model_id,
            tool_call: present(argv, "--jinja").then_some(true),
            reasoning: value(argv, &["--reasoning", "--reasoning-format"])
                .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "none")),
            attachment: traits
                .iter()
                .any(|t| t.capability == Capability::Vision && t.enabled)
                .then_some(true),
            limit: context.map(|context| Limit {
                context,
                output: predict.unwrap_or(context),
            }),
        }
    }

    /// The block as JSON, laid out to be read in a terminal box.
    ///
    /// Written by hand rather than through `serde_json::to_string_pretty`
    /// for two reasons, both about the reader: the key order is the order
    /// OpenCode's own documentation uses (a `serde_json::Map` is a
    /// `BTreeMap` and would sort `$schema` after `provider`), and
    /// `options` stays on one line so the whole block fits an
    /// eighty-by-twenty-four terminal. Validity is not taken on trust —
    /// `the_block_is_valid_json` parses it back.
    pub fn json(&self) -> String {
        let mut model = vec![format!("          \"name\": {}", string(&self.model_id))];
        if let Some(tool_call) = self.tool_call {
            model.push(format!("          \"tool_call\": {tool_call}"));
        }
        if let Some(reasoning) = self.reasoning {
            model.push(format!("          \"reasoning\": {reasoning}"));
        }
        if let Some(attachment) = self.attachment {
            model.push(format!("          \"attachment\": {attachment}"));
        }
        if let Some(limit) = self.limit {
            model.push(format!(
                "          \"limit\": {{ \"context\": {}, \"output\": {} }}",
                limit.context, limit.output
            ));
        }

        let mut out = vec![
            "{".to_string(),
            format!("  \"$schema\": {},", string(SCHEMA)),
            "  \"provider\": {".to_string(),
            format!("    {}: {{", string(PROVIDER_ID)),
            format!("      \"npm\": {},", string(NPM)),
            format!("      \"name\": {},", string(PROVIDER_NAME)),
            format!(
                "      \"options\": {{ \"baseURL\": {} }},",
                string(&format!("{}/v1", self.base_url))
            ),
            "      \"models\": {".to_string(),
            format!("        {}: {{", string(&self.model_id)),
        ];
        out.push(model.join(",\n"));
        out.extend([
            "        }".to_string(),
            "      }".to_string(),
            "    }".to_string(),
            "  }".to_string(),
            "}".to_string(),
        ]);

        out.join("\n")
    }
}

/// A JSON string literal, escaped by the same parser that will read it
/// back. An alias is free text out of a hand-edited ini, and a quote in
/// one must not be able to produce a block that does not parse.
fn string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
}

/// The value following the first of `flags` present in `argv`.
///
/// Several spellings per lookup because the ini decides which one the argv
/// carries: `ctx-size` and `c` are the same option to llama-server and
/// would be two different keys here.
fn value<'a>(argv: &'a [String], flags: &[&str]) -> Option<&'a str> {
    argv.iter().enumerate().find_map(|(index, token)| {
        (flags.contains(&token.as_str()))
            .then(|| argv.get(index + 1))
            .flatten()
            // A flag at the end of the argv, or one immediately followed by
            // another flag, is a valueless flag — not a flag whose value is
            // `--jinja`.
            .filter(|next| !next.starts_with('-'))
            .map(String::as_str)
    })
}

/// Whether a valueless flag is in the argv at all.
fn present(argv: &[String], flag: &str) -> bool {
    argv.iter().any(|token| token == flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|t| t.to_string()).collect()
    }

    fn vision(enabled: bool) -> Vec<Trait> {
        vec![Trait {
            capability: Capability::Vision,
            enabled,
            detail: None,
        }]
    }

    fn sample() -> Provider {
        Provider::from_launch(
            "http://127.0.0.1:1234",
            "gemma4-12b",
            &argv(&[
                "--port",
                "1234",
                "--jinja",
                "--ctx-size",
                "32768",
                "--hf-repo",
                "unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL",
                "--alias",
                "gemma4-12b",
                "--reasoning",
                "off",
            ]),
            &[],
        )
    }

    /// The point of the exercise: OpenCode has to be able to read it.
    /// Hand-written JSON that does not parse is a paste that fails in
    /// another program, hours later, with no clue pointing back here.
    #[test]
    fn the_block_is_valid_json() {
        let parsed: serde_json::Value =
            serde_json::from_str(&sample().json()).expect("the block should parse");

        let model = &parsed["provider"][PROVIDER_ID]["models"]["gemma4-12b"];
        assert_eq!(parsed["$schema"], SCHEMA);
        assert_eq!(parsed["provider"][PROVIDER_ID]["npm"], NPM);
        assert_eq!(
            parsed["provider"][PROVIDER_ID]["options"]["baseURL"],
            "http://127.0.0.1:1234/v1"
        );
        assert_eq!(model["name"], "gemma4-12b");
        assert_eq!(model["tool_call"], true);
        assert_eq!(model["reasoning"], false);
        assert_eq!(model["limit"]["context"], 32768);
    }

    /// `/v1` is appended once, whatever the endpoint arrives as. Pasting
    /// a `baseURL` ending in `/v1/v1` is a silent 404 in another program.
    #[test]
    fn the_endpoint_gets_exactly_one_v1() {
        for base in ["http://127.0.0.1:1234", "http://127.0.0.1:1234/"] {
            let provider = Provider::from_launch(base, "m", &argv(&["--alias", "m"]), &[]);
            let parsed: serde_json::Value = serde_json::from_str(&provider.json()).unwrap();
            assert_eq!(
                parsed["provider"][PROVIDER_ID]["options"]["baseURL"], "http://127.0.0.1:1234/v1",
                "{base}"
            );
        }
    }

    /// The alias is what `/v1/models` answers to, and the preset name is
    /// only a fallback. An ini that sets them differently must produce a
    /// block that works, not one that reads tidily.
    #[test]
    fn the_model_id_is_the_alias_not_the_preset_name() {
        let provider = Provider::from_launch(
            "http://127.0.0.1:1234",
            "qwen3-coder",
            &argv(&["--alias", "qwen-3-coder-30b"]),
            &[],
        );
        assert_eq!(provider.model_id, "qwen-3-coder-30b");

        let no_alias = Provider::from_launch("http://127.0.0.1:1234", "qwen3-coder", &[], &[]);
        assert_eq!(no_alias.model_id, "qwen3-coder");
    }

    /// Nothing herd cannot support is stated. OpenCode then uses its own
    /// defaults, which is a better failure than a confident wrong answer.
    #[test]
    fn a_fact_the_preset_does_not_state_is_left_out() {
        let bare =
            Provider::from_launch("http://127.0.0.1:1234", "m", &argv(&["--alias", "m"]), &[]);

        assert_eq!(bare.tool_call, None, "no --jinja, no tool-call claim");
        assert_eq!(bare.reasoning, None);
        assert_eq!(bare.attachment, None);
        assert_eq!(bare.limit, None, "no --ctx-size, no limit object");

        let json = bare.json();
        for absent in ["tool_call", "reasoning", "attachment", "limit"] {
            assert!(!json.contains(absent), "{absent} was claimed: {json}");
        }
        // ...and leaving fields out must not leave a trailing comma behind.
        serde_json::from_str::<serde_json::Value>(&json).expect("still valid");
    }

    /// Vision is claimed only when the preset switches it on. `no-mmproj`
    /// is set defensively across the shipped tiers, so a preset that *has*
    /// a projector and does not load it cannot take attachments.
    #[test]
    fn attachments_follow_vision_being_switched_on() {
        let on = Provider::from_launch("http://x", "m", &argv(&["--alias", "m"]), &vision(true));
        assert_eq!(on.attachment, Some(true));

        let off = Provider::from_launch("http://x", "m", &argv(&["--alias", "m"]), &vision(false));
        assert_eq!(off.attachment, None, "vision off is not vision");
    }

    /// `off` and `none` are the two spellings the shipped tiers and
    /// llama.cpp's own flag use; anything else means reasoning is on.
    #[test]
    fn reasoning_reads_both_spellings_of_off() {
        for (flag, value, expected) in [
            ("--reasoning", "off", Some(false)),
            ("--reasoning-format", "none", Some(false)),
            ("--reasoning-format", "deepseek", Some(true)),
        ] {
            let provider =
                Provider::from_launch("http://x", "m", &argv(&["--alias", "m", flag, value]), &[]);
            assert_eq!(provider.reasoning, expected, "{flag} {value}");
        }
    }

    /// llama-server generates until the context is full unless the preset
    /// caps it, so that is the ceiling `output` reports — and a real cap
    /// wins over it. Both halves are present or the object is absent,
    /// because OpenCode's schema requires them together.
    #[test]
    fn the_output_limit_is_the_cap_the_server_will_really_enforce() {
        let uncapped = Provider::from_launch(
            "http://x",
            "m",
            &argv(&["--alias", "m", "--ctx-size", "32768"]),
            &[],
        );
        assert_eq!(
            uncapped.limit,
            Some(Limit {
                context: 32768,
                output: 32768
            })
        );

        let capped = Provider::from_launch(
            "http://x",
            "m",
            &argv(&["--alias", "m", "--ctx-size", "32768", "--n-predict", "4096"]),
            &[],
        );
        assert_eq!(
            capped.limit,
            Some(Limit {
                context: 32768,
                output: 4096
            })
        );

        // -1 is llama-server's "no cap" and must not become a limit of one
        // token, nor a negative number in the JSON.
        let unlimited = Provider::from_launch(
            "http://x",
            "m",
            &argv(&["--alias", "m", "--ctx-size", "8192", "--n-predict", "-1"]),
            &[],
        );
        assert_eq!(
            unlimited.limit,
            Some(Limit {
                context: 8192,
                output: 8192
            })
        );
    }

    /// A valueless flag followed by another flag has no value, and reading
    /// the next flag as one would put `--ctx-size` in the model id.
    #[test]
    fn a_flag_with_no_value_is_not_given_the_next_flag_as_one() {
        let provider = Provider::from_launch(
            "http://x",
            "fallback",
            &argv(&["--alias", "--jinja", "--ctx-size", "4096"]),
            &[],
        );
        assert_eq!(provider.model_id, "fallback");
    }

    /// An alias is free text out of a hand-edited file. A quote in one
    /// must escape rather than produce a block that will not parse.
    #[test]
    fn a_quote_in_an_alias_survives_as_json() {
        let provider = Provider::from_launch(
            "http://x",
            "m",
            &argv(&["--alias", r#"it's "the" model"#]),
            &[],
        );
        let parsed: serde_json::Value = serde_json::from_str(&provider.json()).expect("parses");

        assert_eq!(
            parsed["provider"][PROVIDER_ID]["models"][r#"it's "the" model"#]["name"],
            r#"it's "the" model"#
        );
    }

    /// The overlay draws this in a box on a real terminal, so the block
    /// stays compact enough to be worth drawing: `options` on one line
    /// rather than four is what keeps an ordinary preset inside eighty
    /// columns and inside the rows an eighty-by-twenty-four terminal has.
    ///
    /// The height *ceiling* is pinned here rather than left to the
    /// overlay, because it is the JSON's shape that decides it — and the
    /// widest block, with every optional field present, is the one that
    /// has to hold. What the overlay does when the terminal is shorter
    /// than this is its own test.
    #[test]
    fn the_widest_block_stays_compact_enough_to_draw() {
        let widest = Provider {
            base_url: "http://127.0.0.1:1234".into(),
            model_id: "qwen3-vl-8b-instruct".into(),
            tool_call: Some(true),
            reasoning: Some(false),
            attachment: Some(true),
            limit: Some(Limit {
                context: 32768,
                output: 32768,
            }),
        };
        let json = widest.json();
        let lines: Vec<&str> = json.lines().collect();

        assert!(lines.len() <= 19, "{} lines:\n{json}", lines.len());
        for line in lines {
            assert!(
                line.chars().count() <= 76,
                "a {}-column line: {line}",
                line.chars().count()
            );
        }

        // ...and an ordinary preset — the common case — is shorter still.
        assert!(sample().json().lines().count() <= 18);
    }
}
