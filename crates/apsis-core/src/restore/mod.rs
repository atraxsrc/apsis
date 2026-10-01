// SPDX-License-Identifier: GPL-3.0-only

//! Full-system restore (0.5.0): putting `/` back to what a snapshot holds, at the next boot.
//!
//! The design is in `docs/PLAN.md`, Phase 6b. When the copy runs, what it skips and the boot
//! refresh are Apsis's own, not Timeshift's. Everything here works on text and paths it's
//! given, so it's tested without root.

pub mod apsis;
pub mod argv;
pub mod esp;
pub mod file;
pub mod filter;
pub mod plan;
pub mod refusal;
pub mod space;
pub mod state;
