//! Converts the original game's characters into this project's RON format.
//!
//! Pipeline, per character: `extract` reads the decompiled scenes and scripts into structured
//! data, and `convert` turns that into a character file, an effects file and a report of what
//! could not be expressed.

pub mod controls;
pub mod convert;
pub mod extract;
pub mod pipeline;
pub mod ron;
pub mod script;
pub mod tscn;
pub mod verified;

pub use convert::{convert_character, convert_character_with, Converted};
