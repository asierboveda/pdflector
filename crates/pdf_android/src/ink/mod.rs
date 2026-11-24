// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Causal low-latency stylus ink engine for Android (ADR-009 / ADR-010).
//!
//! Only real pen samples are emitted (`causal` / `engine`), with no forward
//! extrapolation.

pub mod causal;
pub mod engine;

pub use causal::{CausalEngine, CausalInkEngine};
pub use engine::{
    DirtyRect, InkDelta, InkEngine, InkError, InkFinal, InkRange, InkSample, InkStyle,
};

#[cfg(test)]
mod contract_tests;
