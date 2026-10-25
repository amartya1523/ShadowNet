//! The moving half of ShadowNet.
//!
//! Control plane: discrete-event, exact. Every LSA, BGP update and SPF run
//! is an event with a timestamp.
//!
//! Data plane: fluid. Each tick, every flow walks the forwarding state of
//! the routers it actually visits, its demand lands on the links it crosses,
//! and the link queues integrate. See docs/DESIGN.md for why that split.

pub mod bgp;
pub mod brain;
pub mod chaos;
pub mod event;
pub mod forward;
pub mod igp;
pub mod links;
pub mod scenario;
pub mod time;
pub mod traffic;
pub mod wire;
pub mod world;

pub use scenario::Scenario;
pub use traffic::{FlowKind, PathStatus};
pub use wire::Command;
pub use world::{RoutingMode, SimConfig, World};
