//! The static half of ShadowNet: what the network looks like before anything
//! happens to it, and the textbook algorithms everything else leans on.
//!
//! Nothing in here knows about time. `shadow-sim` owns the clock.

pub mod algo;
pub mod geo;
pub mod ids;
pub mod rng;
pub mod topology;

pub use ids::{AsId, CityId, HostId, LinkId, RouterId, ServiceId};
pub use rng::Rng;
pub use topology::{
    AutonomousSystem, Link, LinkKind, Relationship, Router, RouterRole, Service, Tier, Topology,
    TopologyConfig,
};
