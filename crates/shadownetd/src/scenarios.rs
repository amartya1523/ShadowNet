//! Scenario loading. The repository's `scenarios/` directory is compiled in,
//! so the binary works from anywhere; files in a directory passed with
//! `--scenarios` override built-ins of the same name.

use std::path::Path;

use anyhow::{Context, Result};
use shadow_sim::Scenario;
use tracing::info;

const BUILTIN: &[(&str, &str)] = &[
    ("transatlantic-cut", include_str!("../../../scenarios/transatlantic-cut.toml")),
    ("flash-crowd", include_str!("../../../scenarios/flash-crowd.toml")),
    ("ddos", include_str!("../../../scenarios/ddos.toml")),
    ("cloud-outage", include_str!("../../../scenarios/cloud-outage.toml")),
    ("tier1-down", include_str!("../../../scenarios/tier1-down.toml")),
    ("router-crash", include_str!("../../../scenarios/router-crash.toml")),
    ("chaos-monkey", include_str!("../../../scenarios/chaos-monkey.toml")),
    ("brownout", include_str!("../../../scenarios/brownout.toml")),
];

pub fn parse(name: &str, text: &str) -> Result<Scenario> {
    toml::from_str(text).with_context(|| format!("parsing scenario {name}"))
}

pub fn load(dir: Option<&Path>) -> Result<Vec<Scenario>> {
    let mut out: Vec<Scenario> = BUILTIN.iter().map(|(name, text)| parse(name, text)).collect::<Result<_>>()?;
    if let Some(dir) = dir {
        for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let text = std::fs::read_to_string(&path)?;
            let s = parse(&path.display().to_string(), &text)?;
            info!(scenario = %s.name, path = %path.display(), "loaded scenario");
            out.retain(|x| x.name != s.name);
            out.push(s);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn builtins_parse() {
        let all = super::load(None).unwrap();
        assert_eq!(all.len(), super::BUILTIN.len());
        for s in &all {
            assert!(!s.steps.is_empty(), "{} has no steps", s.name);
            assert!(s.steps.iter().all(|st| st.at <= s.duration), "{} has a step after its end", s.name);
        }
    }
}
