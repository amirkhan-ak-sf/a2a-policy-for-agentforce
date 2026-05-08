//! A2A 0.3.0 protocol surface.
//!
//! `types`   - the protocol data model (Task, Message, Part, ...).
//! `mapping` - converters between A2A objects and Agentforce request/response shapes.
//! `methods` - JSON-RPC method dispatch (`message/send`, `tasks/get`, `tasks/cancel`).

pub mod mapping;
pub mod methods;
pub mod types;
