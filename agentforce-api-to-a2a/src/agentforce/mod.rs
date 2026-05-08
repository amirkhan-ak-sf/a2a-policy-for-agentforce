//! Agentforce client.
//!
//! `auth` mints OAuth access tokens via the connected app's
//! `client_credentials` grant; `client` performs the three Agent API calls
//! the policy needs (start session, sync send message, end session) with a
//! single 401-driven token-refresh retry.

pub mod auth;
pub mod client;
