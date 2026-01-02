//! Memory Server - A context memory governance service for Agent/LLM applications
//!
//! This crate provides structured memory storage, retrieval, evolution and governance
//! capabilities without calling LLM inference, binding to models, or invading business logic.

pub mod api;
pub mod config;
pub mod domain;
pub mod embedding;
pub mod error;
pub mod llm;
pub mod repository;
pub mod service;
