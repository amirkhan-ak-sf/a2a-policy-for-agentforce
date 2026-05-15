//! Anypoint Exchange asset publisher.
//!
//! Pushes the resolved agent-card.json to Exchange as a new version of an
//! existing `agent` asset. Driven by the `publishAgentCardToExchange`
//! toggle in the policy config.
//!
//! Failures at any step are logged at warn and never block traffic.

pub mod multipart;
pub mod publish;
