//! `domain::ports` — ports (interfaces) the application core depends on.
//!
//! Adapters (in `crate::adapters`) implement these ports. The application
//! core never depends on a concrete adapter — only on these traits (hexagonal
//! dependency inversion, ARCHITECTURE-SPINE "Design Paradigm").

pub mod provider_adapter;
pub mod index_store;
pub mod query_store;
