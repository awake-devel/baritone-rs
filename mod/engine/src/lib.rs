// baritone-rs engine. Baritone's pathing rules in plain Rust, with no game and no dependencies.
// Ported to Rust from cabaletta/baritone at commit 1b8e87cac675d72d8094e2bea1ec7207cdbc971b. See CREDITS.md.
//
// This file is part of Baritone.
//
// Baritone is free software: you can redistribute it and/or modify
// it under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// Baritone is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Lesser General Public License for more details.
//
// You should have received a copy of the GNU Lesser General Public License
// along with Baritone.  If not, see <https://www.gnu.org/licenses/>.
#![forbid(unsafe_code)]

pub mod blocks;
pub mod config;
pub mod costs;
pub mod goals;
pub mod movement;
pub mod plan;
pub mod pos;
pub mod search;
pub mod world;

#[cfg(test)]
mod testing;
