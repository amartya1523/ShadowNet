//! Scripted experiments.
//!
//! A scenario is a list of timed steps. Steps never name link or router ids
//! (those change with the seed); they name *intent*: "cut everything between
//! North America and Europe", "crash the busiest router", "send a flash
//! crowd at the least popular service". The world resolves intent against
//! whatever topology it is running.

use serde::Deserialize;

use crate::world::RoutingMode;

#[derive(Clone, Debug, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub title: String,
    pub description: String,
    pub duration: f64,
    #[serde(default)]
    pub routing: Option<RoutingMode>,
    #[serde(default, rename = "step")]
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Step {
    /// Seconds after the scenario starts.
    pub at: f64,
    #[serde(flatten)]
    pub action: Action,
}

/// Which service a step is aimed at: a name, or one of `@top` (most
/// popular), `@tail` (least popular), `@single` (most popular one that only
/// lives in one datacenter, which is the one a flash crowd can actually
/// hurt).
pub type ServiceSel = String;

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Action {
    Note {
        text: String,
    },
    CutBetweenRegions {
        from: String,
        to: String,
        #[serde(default)]
        max: Option<usize>,
    },
    CutBusiest {
        count: usize,
    },
    CrashBusiestRouter,
    /// Take down the hosting AS behind a service's primary replica.
    FailHostingAs {
        service: ServiceSel,
    },
    /// Take down the tier-1 with the most customers.
    FailBiggestTier1,
    FlashCrowd {
        service: ServiceSel,
        users: usize,
        mbps: f64,
        #[serde(default)]
        ramp: f64,
        secs: f64,
    },
    Ddos {
        service: ServiceSel,
        bots: usize,
        mbps: f64,
        secs: f64,
    },
    LatencySpikeBetweenRegions {
        from: String,
        to: String,
        ms: f64,
        secs: f64,
    },
    SetRouting {
        mode: RoutingMode,
    },
    ChaosMonkey {
        enabled: bool,
        #[serde(default)]
        per_minute: Option<f64>,
    },
    HealAll,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_shape() {
        // The server parses TOML; the structure is the same, and JSON keeps
        // this crate free of a toml dependency.
        let s: Scenario = serde_json::from_str(
            r#"{
                "name": "x", "title": "X", "description": "d", "duration": 60,
                "step": [
                    {"at": 1, "do": "note", "text": "hello"},
                    {"at": 5, "do": "cut_between_regions", "from": "NA", "to": "EU"},
                    {"at": 9, "do": "flash_crowd", "service": "@single", "users": 10000, "mbps": 8, "secs": 30},
                    {"at": 12, "do": "set_routing", "mode": "qrouting"},
                    {"at": 20, "do": "heal_all"}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(s.steps.len(), 5);
        assert!(matches!(s.steps[3].action, Action::SetRouting { mode: RoutingMode::Qrouting }));
    }
}
